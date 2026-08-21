//! Integration tests for the JSON Schema 2020-12 validator.
//!
//! Schema and instance are embedded in one parsed document (the frozen
//! `Schema::validate(&self, NodeRef<'d>)` signature ties both to the same
//! document lifetime, mirroring linter usage where instances are parts of
//! the same parsed document as the schemas).

use std::path::Path;

use suspect_low::LowDoc;
use suspect_source::{Source, Uri};
use suspect_schema::{CompileError, Compiler, Config, SchemaError};

fn parse(text: &str) -> LowDoc {
    let uri = Uri::from_path(Path::new("/t/schema.json")).expect("uri");
    LowDoc::parse(uri, Source::from_vec(text.as_bytes().to_vec()))
}

/// Compiles `$schema`/`$instance` embedded in one document and binds
/// `$doc`, `$sch`, `$ins` in the caller's scope (the document must outlive
/// both compiled validator and instance view).
macro_rules! bind_case {
    ($cfg:expr, $schema:expr, $instance:expr, $doc:ident, $sch:ident, $ins:ident) => {
        let $doc =
            parse(&format!("{{\"S\": {}, \"I\": {}}}", $schema, $instance));
        let $sch = Compiler::new($cfg)
            .compile($doc.root().get("S").expect("schema slot"))
            .expect("compile");
        let $ins = $doc.root().get("I").expect("instance slot");
    };
}

fn valid(schema: &str, instance: &str) -> bool {
    bind_case!(Config::default(), schema, instance, doc, s, inst);
    s.validate(inst).is_empty()
}

fn invalid(schema: &str, instance: &str) -> Vec<SchemaError> {
    bind_case!(Config::default(), schema, instance, doc, s, inst);
    s.validate(inst)
}

// ---------------------------------------------------------------------------
// type / enum / const
// ---------------------------------------------------------------------------

#[test]
fn type_keyword_matches_json_types() {
    assert!(valid(r##"{"type":"integer"}"##, "42"));
    assert!(!valid(r##"{"type":"integer"}"##, "4.5"));
    assert!(valid(r##"{"type":"integer"}"##, "4.0")); // zero fractional part
    assert!(valid(r##"{"type":"number"}"##, "4.5"));
    assert!(!valid(r##"{"type":"number"}"##, "true"));
    assert!(valid(r##"{"type":["string","null"]}"##, "null"));
    assert!(valid(r##"{"type":["string","null"]}"##, "\"x\""));
    assert!(!valid(r##"{"type":["string","null"]}"##, "3"));
    assert!(valid(r##"{"type":"boolean"}"##, "true"));
    assert!(valid(r##"{"type":"array"}"##, "[1]"));
    assert!(valid(r##"{"type":"object"}"##, "{}"));
}

#[test]
fn enum_deep_equality() {
    assert!(valid(
        r##"{"enum":[1,{"a":[1,2]},"x"]}"##,
        r##"{"a":[1,2]}"##
    ));
    assert!(!valid(r##"{"enum":[1,{"a":[1,2]},"x"]}"##, r##"{"a":[2,1]}"##));
    assert!(!valid(r##"{"enum":[1,{"a":[1,2]},"x"]}"##, r##"{"a":[1,2],"b":3}"##));
    // numeric equality across int/float representations
    assert!(valid(r##"{"enum":[1.0]}"##, "1"));
    assert!(valid(r##"{"enum":[1]}"##, "1.0"));
    // order-insensitive objects
    assert!(valid(r##"{"enum":[{"a":1,"b":2}]}"##, r##"{"b":2,"a":1}"##));
}

#[test]
fn const_keyword() {
    assert!(valid(r##"{"const":{"k":[true,null]}}"##, r##"{"k":[true,null]}"##));
    assert!(!valid(r##"{"const":{"k":[true,null]}}"##, r##"{"k":[null,true]}"##));
    assert!(valid(r##"{"const":0}"##, "0.0"));
    assert!(!valid(r##"{"const":0}"##, "1"));
}

// ---------------------------------------------------------------------------
// numeric
// ---------------------------------------------------------------------------

#[test]
fn multiple_of_float_precision() {
    // classic binary-float trap: 0.07 is a "multiple" of 0.01
    assert!(valid(r##"{"multipleOf":0.01}"##, "0.07"));
    assert!(valid(r##"{"multipleOf":0.01}"##, "0.05"));
    assert!(!valid(r##"{"multipleOf":0.01}"##, "0.073"));
    assert!(valid(r##"{"multipleOf":0.1}"##, "0.3"));
    assert!(valid(r##"{"multipleOf":7}"##, "-21"));
    assert!(!valid(r##"{"multipleOf":7}"##, "20"));
    assert!(valid(r##"{"multipleOf":1.5}"##, "4.5"));
    // zero fractional floats take the exact integer path
    assert!(valid(r##"{"multipleOf":2.0}"##, "10.0"));
}

#[test]
fn numeric_bounds_and_exclusive_forms() {
    assert!(valid(r##"{"maximum":5}"##, "5"));
    assert!(valid(r##"{"maximum":5}"##, "5.0"));
    assert!(!valid(r##"{"maximum":5}"##, "5.5"));
    assert!(!valid(r##"{"exclusiveMaximum":5}"##, "5"));
    assert!(valid(r##"{"exclusiveMaximum":5}"##, "4.9"));
    assert!(valid(r##"{"minimum":-3}"##, "-3"));
    assert!(!valid(r##"{"exclusiveMinimum":-3}"##, "-3"));
    assert!(valid(r##"{"exclusiveMinimum":-3}"##, "-2.9"));
    // 2020-12 removed the boolean form entirely
    for kw in ["exclusiveMinimum", "exclusiveMaximum"] {
        let text =
            format!(r##"{{"S": {{"{kw}": true}}, "I": 1}}"##);
        let doc = parse(&text);
        match Compiler::new(Config::default()).compile(doc.root().get("S").expect("s")) {
            Err(CompileError::Invalid { .. }) => {}
            _other => panic!("bool form must be a compile error"),
        }
    }
}

// ---------------------------------------------------------------------------
// strings
// ---------------------------------------------------------------------------

#[test]
fn string_length_counts_unicode_scalars() {
    // two astral-plane scalars = 2 chars, 8 bytes
    assert!(valid(r##"{"maxLength":2}"##, "\"\u{1D11E}\u{1D11E}\""));
    assert!(!valid(r##"{"maxLength":2}"##, "\"\u{1D11E}\u{1D11E}\u{1D11E}\""));
    assert!(valid(r##"{"minLength":2}"##, "\"\u{1D11E}\u{1D11E}\""));
    assert!(!valid(r##"{"minLength":3}"##, "\"\u{1D11E}\u{1D11E}\""));
    // non-strings pass string keywords vacuously
    assert!(valid(r##"{"maxLength":1}"##, "12345"));
}

#[test]
fn pattern_keyword() {
    assert!(valid(r##"{"pattern":"^a+$"}"##, "\"aaa\""));
    assert!(!valid(r##"{"pattern":"^a+$"}"##, "\"aab\""));
    assert!(valid(r##"{"pattern":"b"}"##, "\"abc\""));
}

// ---------------------------------------------------------------------------
// arrays
// ---------------------------------------------------------------------------

#[test]
fn items_and_prefix_items() {
    assert!(valid(r##"{"items":{"type":"integer"}}"##, "[1,2,3]"));
    assert!(!valid(r##"{"items":{"type":"integer"}}"##, "[1,\"x\"]"));
    assert!(valid(r##"{"items":{"type":"integer"}}"##, "[]"));
    // 2020-12 items applies to ALL elements (no tuple form)
    assert!(valid(
        r##"{"prefixItems":[{"type":"string"}],"items":{"type":"integer"}}"##,
        r##"["a",1,2]"##
    ));
    assert!(!valid(
        r##"{"prefixItems":[{"type":"string"}],"items":{"type":"integer"}}"##,
        r##"["a",1,"c"]"##
    ));
    assert!(!valid(
        r##"{"prefixItems":[{"type":"integer"}]}"##,
        r##"["a"]"##
    ));
    // array-form `items` is a compile error in 2020-12
    let doc = parse(r##"{"S":{"items":[{"type":"integer"}]}}"##);
    assert!(matches!(
        Compiler::new(Config::default()).compile(doc.root().get("S").expect("s")),
        Err(CompileError::Invalid { .. })
    ));
}

#[test]
fn contains_bounds() {
    assert!(valid(r##"{"contains":{"const":2}}"##, "[1,2,3]"));
    assert!(!valid(r##"{"contains":{"const":2}}"##, "[1,3]"));
    assert!(valid(r##"{"contains":{"const":2}}"##, "[2]"));
    assert!(valid(r##"{"contains":{"const":2},"minContains":0}"##, "[1,3]"));
    assert!(!valid(
        r##"{"contains":{"const":2},"minContains":2}"##,
        "[1,2,3]"
    ));
    assert!(!valid(
        r##"{"contains":{"const":2},"maxContains":1}"##,
        "[2,2]"
    ));
    assert!(valid(
        r##"{"contains":{"const":2},"maxContains":2}"##,
        "[2,2,3]"
    ));
    // minContains > 0 fails for non-arrays too
    assert!(!valid(r##"{"contains":{"const":1}}"##, "{}"));
}

// ---------------------------------------------------------------------------
// objects
// ---------------------------------------------------------------------------

#[test]
fn properties_and_patterns() {
    let schema = r##"{
        "properties": {"name": {"type": "string"}},
        "patternProperties": {"^n": {"type": "string"}},
        "additionalProperties": {"type": "integer"}
    }"##;
    assert!(valid(schema, r##"{"name":"a","nick":"b","extra":3}"##));
    assert!(!valid(schema, r##"{"name":"a","extra":"s"}"##));
    assert!(!valid(schema, r##"{"name":1}"##));

    // additionalProperties: false with sibling exceptions
    let closed = r##"{
        "properties": {"a": true},
        "patternProperties": {"^b": true},
        "additionalProperties": false
    }"##;
    assert!(valid(closed, r##"{"a":1,"bb":2}"##));
    assert!(!valid(closed, r##"{"a":1,"c":2}"##));
    // order-independence: additionalProperties declared FIRST
    let closed2 = r##"{
        "additionalProperties": false,
        "properties": {"a": true}
    }"##;
    assert!(valid(closed2, r##"{"a":1}"##));
    assert!(!valid(closed2, r##"{"a":1,"z":2}"##));
}

#[test]
fn property_names_keyword() {
    let schema = r##"{"propertyNames":{"pattern":"^[a-z]+$"}}"##;
    assert!(valid(schema, r##"{"abc":1}"##));
    assert!(!valid(schema, r##"{"Abc":1}"##));
    assert!(valid(schema, "{}"));
    let len_schema = r##"{"propertyNames":{"maxLength":2}}"##;
    assert!(valid(len_schema, r##"{"ab":1}"##));
    assert!(!valid(len_schema, r##"{"abc":1}"##));
}

#[test]
fn dependent_required_and_dependencies() {
    let dr = r##"{"dependentRequired":{"credit_card":["billing_address"]}}"##;
    assert!(valid(dr, r##"{"credit_card":1,"billing_address":"x"}"##));
    assert!(valid(dr, r##"{"billing_address":"x"}"##));
    assert!(!valid(dr, r##"{"credit_card":1}"##));

    // legacy `dependencies`: array form == dependentRequired
    let dep = r##"{"dependencies":{"credit_card":["billing_address"]}}"##;
    assert!(valid(dep, r##"{"credit_card":1,"billing_address":"x"}"##));
    assert!(!valid(dep, r##"{"credit_card":1}"##));

    // legacy `dependencies`: object form == dependentSchemas
    let dep2 =
        r##"{"dependencies":{"credit_card":{"properties":{"billing_address":{"type":"string"}},"required":["billing_address"]}}}"##;
    assert!(valid(dep2, r##"{"credit_card":1,"billing_address":"y"}"##));
    assert!(!valid(dep2, r##"{"credit_card":1}"##));
}

#[test]
fn dependent_schemas_keyword() {
    let schema = r##"{
        "properties": {"a": true},
        "dependentSchemas": {
            "a": {"required": ["b"]}
        }
    }"##;
    assert!(valid(schema, r##"{"a":1,"b":2}"##));
    assert!(valid(schema, r##"{"c":1}"##));
    assert!(!valid(schema, r##"{"a":1}"##));
}

// ---------------------------------------------------------------------------
// unevaluated* (annotation tracking)
// ---------------------------------------------------------------------------

#[test]
fn unevaluated_properties_composition_suite() {
    // 1. sibling `properties` evaluates
    let s1 = r##"{"properties":{"foo":{"type":"string"}},"unevaluatedProperties":false}"##;
    assert!(valid(s1, r##"{"foo":"a"}"##));
    let e = invalid(s1, r##"{"foo":"a","bar":1}"##);
    assert_eq!(e.len(), 1);
    assert!(e[0].message.contains("unevaluated"));

    // 2. patternProperties evaluates
    let s2 = r##"{"patternProperties":{"^s_":true},"unevaluatedProperties":false}"##;
    assert!(valid(s2, r##"{"s_a":1,"s_b":2}"##));
    assert!(!valid(s2, r##"{"s_a":1,"t":2}"##));

    // 3. additionalProperties:true evaluates everything
    let s3 = r##"{"additionalProperties":{},"unevaluatedProperties":false}"##;
    assert!(valid(s3, r##"{"x":1,"y":[1,2]}"##));

    // 4. allOf branch evaluations count
    let s4 = r##"{"allOf":[{"properties":{"a":true}}],"unevaluatedProperties":false}"##;
    assert!(valid(s4, r##"{"a":1}"##));
    assert!(!valid(s4, r##"{"a":1,"b":2}"##));

    // 5. anyOf passing branches count (both, when both pass)
    let s5 = r##"{
        "anyOf": [
            {"properties":{"foo":true},"required":["foo"]},
            {"properties":{"bar":true},"required":["bar"]}
        ],
        "unevaluatedProperties": false
    }"##;
    assert!(valid(s5, r##"{"foo":1}"##));
    assert!(valid(s5, r##"{"foo":1,"bar":2}"##));
    assert!(!valid(s5, r##"{"baz":1}"##));

    // 6. dependentSchemas inner evaluations count
    let s6 = r##"{
        "properties": {"credit_card": true},
        "dependentSchemas": {
            "credit_card": {"properties": {"billing": true}, "required": ["billing"]}
        },
        "unevaluatedProperties": false
    }"##;
    assert!(valid(s6, r##"{"credit_card":1,"billing":"x"}"##));
    assert!(!valid(s6, r##"{"credit_card":1}"##));

    // 7. failing sibling leaves the property unevaluated (record-on-pass)
    let s7 = r##"{"properties":{"a":{"type":"string"}},"unevaluatedProperties":false}"##;
    let e7 = invalid(s7, r##"{"a":1}"##);
    assert!(e7.iter().any(|x| x.message.contains("unevaluated")));

    // 8. nested unevaluatedProperties inside allOf branch also evaluates
    let s8 = r##"{
        "allOf": [
            {"properties": {"a": true}, "unevaluatedProperties": false}
        ],
        "unevaluatedProperties": false
    }"##;
    assert!(valid(s8, r##"{"a":1}"##));
    assert!(!valid(s8, r##"{"b":1}"##));
}

#[test]
fn unevaluated_items_suite() {
    // items evaluates every index
    let s1 = r##"{"items":true,"unevaluatedItems":false}"##;
    assert!(valid(s1, "[1,2,3]"));

    // prefixItems evaluates only the prefix
    let s2 = r##"{"prefixItems":[{},{}],"unevaluatedItems":false}"##;
    assert!(valid(s2, "[1,2]"));
    let e = invalid(s2, "[1,2,3]");
    assert!(e.iter().any(|x| x.schema_path.to_path().contains("unevaluatedItems")));

    // contains evaluates only matched indices
    let s3 = r##"{"contains":{"multipleOf":2},"unevaluatedItems":false}"##;
    let e3 = invalid(s3, "[1,2,3]");
    assert_eq!(e3.len(), 2); // indices 0 and 2 unevaluated
    assert!(valid(s3, "[2]"));

    // composition: prefixItems + contains together
    let s4 = r##"{"prefixItems":[{}],"contains":{"const":2},"unevaluatedItems":false}"##;
    assert!(valid(s4, "[1,2]")); // idx0 by prefixItems, idx1 by contains
    assert!(!valid(s4, "[3,2,1]"));
}

// ---------------------------------------------------------------------------
// composition
// ---------------------------------------------------------------------------

#[test]
fn all_of_any_of_one_of_not() {
    assert!(valid(r##"{"allOf":[{"type":"integer"},{"minimum":0}]}"##, "3"));
    assert!(!valid(r##"{"allOf":[{"type":"integer"},{"minimum":0}]}"##, "-3"));

    assert!(valid(r##"{"anyOf":[{"type":"string"},{"type":"integer"}]}"##, "3"));
    assert!(!valid(r##"{"anyOf":[{"type":"string"},{"type":"boolean"}]}"##, "3"));

    assert!(valid(r##"{"oneOf":[{"type":"string"},{"type":"integer"}]}"##, "3"));
    assert!(!valid(
        r##"{"oneOf":[{"type":"number"},{"type":"integer"}]}"##,
        "3"
    )); // both match
    assert!(!valid(r##"{"oneOf":[{"type":"string"},{"type":"boolean"}]}"##, "3"));

    assert!(valid(r##"{"not":{"type":"string"}}"##, "3"));
    assert!(!valid(r##"{"not":{"type":"integer"}}"##, "3"));
}

#[test]
fn any_of_one_of_error_reporting_lists_branches() {
    let e = invalid(
        r##"{"anyOf":[{"type":"string"},{"minimum":10}]}"##,
        "3",
    );
    assert_eq!(e.len(), 1);
    let msg = &e[0].message;
    assert!(msg.contains("does not match any `anyOf` branch"), "{msg}");
    assert!(msg.contains("branch 0"), "{msg}");
    assert!(msg.contains("branch 1"), "{msg}");

    let e2 = invalid(
        r##"{"oneOf":[{"type":"number"},{"type":"integer"}]}"##,
        "3",
    );
    assert!(e2[0].message.contains("matches 2 `oneOf` branches"), "{:?}", e2[0].message);
}

#[test]
fn if_then_else() {
    let s = r##"{
        "if": {"properties":{"kind":{"const":"a"}},"required":["kind"]},
        "then": {"required":["a_val"]},
        "else": {"required":["b_val"]}
    }"##;
    assert!(valid(s, r##"{"kind":"a","a_val":1}"##));
    assert!(!valid(s, r##"{"kind":"a"}"##));
    assert!(valid(s, r##"{"kind":"b","b_val":1}"##));
    assert!(!valid(s, r##"{"kind":"b"}"##));
    // no `kind` -> else branch
    assert!(valid(s, r##"{"b_val":2}"##));
    assert!(!valid(s, "{}"));
    // `if` without then/else asserts nothing
    assert!(valid(r##"{"if":{"type":"string"}}"##, "3"));
}

// ---------------------------------------------------------------------------
// $ref: recursion, anchors, $id, external
// ---------------------------------------------------------------------------

fn deep_tree(depth: usize) -> String {
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str("{\"value\":1,\"children\":[");
    }
    s.push_str("{\"value\":1,\"children\":[]}");
    for _ in 0..depth {
        s.push_str("]}");
    }
    s
}

#[test]
fn recursive_ref_validates_1000_deep_instance() {
    // debug builds have fat frames; give the test thread headroom.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(recursive_ref_validates_1000_deep_instance_inner)
        .expect("spawn");
    handle.join().expect("inner");
}

fn recursive_ref_validates_1000_deep_instance_inner() {
    let schema = r##"{
        "type": "object",
        "properties": {
            "value": {"type": "integer"},
            "children": {"items": {"$ref": "#"}}
        },
        "required": ["value", "children"]
    }"##;
    let inst = deep_tree(1000);
    let cfg = Config { max_depth: 8192, ..Config::default() };
    bind_case!(cfg.clone(), schema, &inst, _doc, s, i);
    let errors = s.validate(i);
    assert!(errors.is_empty(), "deep valid tree must validate: {:?}", errors.first());

    // a bad leaf deep inside must be found
    let mut bad = String::new();
    for _ in 0..500 {
        bad.push_str("{\"value\":1,\"children\":[");
    }
    bad.push_str("{\"value\":\"oops\",\"children\":[]}");
    for _ in 0..500 {
        bad.push_str("]}");
    }
    bind_case!(cfg.clone(), schema, &bad, _doc2, s2, i2);
    let errors2 = s2.validate(i2);
    assert!(!errors2.is_empty());
    assert!(errors2[0].instance_path.to_path().contains("value"));
}

#[test]
fn depth_cap_returns_clean_error_not_crash() {
    let schema =
        r##"{"type":"object","properties":{"children":{"items":{"$ref":"#"}}}}"##;
    let inst = deep_tree(50);
    let cfg = Config { max_depth: 8, ..Config::default() };
    bind_case!(cfg, schema, &inst, _doc, s, i);
    let errors = s.validate(i);
    assert!(!errors.is_empty());
    assert!(errors[0].message.contains("depth exceeds"));
}

#[test]
fn ref_to_defs_and_anchor() {
    // pointer ref into $defs
    let s1 = r##"{"$defs":{"pos":{"type":"integer","minimum":0}},"properties":{"x":{"$ref":"#/$defs/pos"}}}"##;
    assert!(valid(s1, r##"{"x":5}"##));
    assert!(!valid(s1, r##"{"x":-1}"##));

    // plain-name fragment via $anchor
    let s2 = r##"{
        "$defs": {"positive": {"$anchor": "positive", "type": "integer", "minimum": 1}},
        "properties": {"x": {"$ref": "#positive"}}
    }"##;
    assert!(valid(s2, r##"{"x":5}"##));
    assert!(!valid(s2, r##"{"x":0}"##));

    // unknown anchor is a compile error
    let doc = parse(r##"{"S":{"properties":{"x":{"$ref":"#nope"}}}}"##);
    assert!(matches!(
        Compiler::new(Config::default()).compile(doc.root().get("S").expect("s")),
        Err(CompileError::Invalid { .. })
    ));
}

#[test]
fn external_ref_yields_clean_exec_error() {
    let s = r##"{"$ref": "pet.yaml#/components/schemas/Pet"}"##;
    let e = invalid(s, "3");
    assert_eq!(e.len(), 1);
    assert_eq!(
        e[0].message, "external schema resolution not configured",
        "exact message required"
    );
    // relative refs that leave the document are external too
    let e2 = invalid(r##"{"$ref":"../common/b.yaml#/A"}"##, "3");
    assert_eq!(e2[0].message, "external schema resolution not configured");
}

#[test]
fn id_base_registration() {
    // root $id; same-document refs still work
    let s1 = r##"{
        "$id": "https://example.com/root.json",
        "$defs": {"t": {"type": "string"}},
        "properties": {"x": {"$ref": "#/$defs/t"}}
    }"##;
    assert!(valid(s1, r##"{"x":"ok"}"##));

    // a ref resolving back to THIS document through its $id is local
    let s2 = r##"{
        "$id": "https://example.com/root.json",
        "$defs": {"t": {"type": "string"}},
        "properties": {"x": {"$ref": "https://example.com/root.json#/$defs/t"}}
    }"##;
    assert!(valid(s2, r##"{"x":"ok"}"##));

    // nested $id re-roots the base: a ref to the nested resource is local,
    // a ref to a foreign document is external
    let s3 = r##"{
        "$id": "https://example.com/root.json",
        "$defs": {
            "nested": {
                "$id": "nested.json",
                "$defs": {"t": {"type": "integer"}},
                "properties": {"x": {"$ref": "https://example.com/nested.json#/$defs/t"}}
            }
        },
        "properties": {"y": {"$ref": "#/$defs/nested/properties/x"}}
    }"##;
    assert!(valid(s3, r##"{"y":7}"##));
    assert!(!valid(s3, r##"{"y":"s"}"##));

    let s4 = r##"{
        "$id": "https://example.com/root.json",
        "$defs": {
            "nested": {
                "$id": "nested.json",
                "properties": {"x": {"$ref": "https://elsewhere.org/other.json#/$defs/t"}}
            }
        },
        "properties": {"y": {"$ref": "#/$defs/nested/properties/x"}}
    }"##;
    let e = invalid(s4, r##"{"y":7}"##);
    assert_eq!(e[0].message, "external schema resolution not configured");
}

#[test]
fn dynamic_ref_basic_rfc3093() {
    // Outermost dynamic scope wins: the root declares the anchor, so both
    // leaves validate against the ROOT schema (extensible trees).
    let s = r##"{
        "$dynamicAnchor": "node",
        "type": "object",
        "properties": {
            "value": {"type": "integer"},
            "children": {"items": {"$ref": "#/$defs/leaf"}}
        },
        "$defs": {
            "leaf": {
                "$dynamicAnchor": "node",
                "type": "object",
                "properties": {
                    "value": {"type": "integer"},
                    "children": {"items": {"$dynamicRef": "#node"}}
                }
            }
        }
    }"##;
    assert!(valid(s, r##"{"value":1,"children":[{"value":2,"children":[]}]}"##));
    assert!(!valid(s, r##"{"value":1,"children":[{"value":"x","children":[]}]}"##));
    // deep nesting still resolves through the dynamic scope to the root
    assert!(valid(
        s,
        r##"{"value":1,"children":[{"value":2,"children":[{"value":3}]}]}"##
    ));

    // $dynamicRef with no dynamic scope and no registry entry errors cleanly
    let s2 = r##"{"properties": {"x": {"$dynamicRef": "#nowhere"}}}"##;
    let e = invalid(s2, r##"{"x":1}"##);
    assert!(!e.is_empty() && e[0].message.contains("unresolvable $dynamicRef"));
}

// ---------------------------------------------------------------------------
// format
// ---------------------------------------------------------------------------

#[test]
fn format_assertion_on_and_off() {
    let schema = r##"{"format":"date-time"}"##;
    // off (default): annotation-only
    assert!(valid(schema, "\"not a date\""));
    // on: asserted
    let cfg = Config { format_assertion: true, ..Config::default() };
    let ok = |inst: &str| {
        let text = format!("{{\"S\": {schema}, \"I\": {inst}}}");
        let doc = parse(&text);
        let root = doc.root();
        let s = Compiler::new(cfg.clone()).compile(root.get("S").expect("s")).expect("c");
        s.validate(root.get("I").expect("i")).is_empty()
    };
    assert!(ok("\"2020-12-31T23:59:60Z\""));
    assert!(ok("\"2020-02-29T12:00:00.5+05:30\""));
    assert!(!ok("\"not a date\""));
    assert!(!ok("\"2020-13-01T00:00:00Z\""));
    assert!(!ok("\"2021-02-29T00:00:00Z\""));

    let f = |fmt: &str, inst: &str| {
        let text = format!(r##"{{"S": {{"format":"{fmt}"}}, "I": {inst}}}"##);
        let doc = parse(&text);
        let root = doc.root();
        let s = Compiler::new(Config { format_assertion: true, ..Config::default() })
            .compile(root.get("S").expect("s"))
            .expect("c");
        s.validate(root.get("I").expect("i")).is_empty()
    };
    assert!(f("date", "\"2024-02-29\""));
    assert!(!f("date", "\"2023-02-29\""));
    assert!(f("time", "\"23:59:59Z\""));
    assert!(!f("time", "\"24:00:00Z\""));
    assert!(f("email", "\"a.b-c@example.co\""));
    assert!(!f("email", "\"@bar\""));
    assert!(f("hostname", "\"example.com\""));
    assert!(!f("hostname", "\"-bad-.com\""));
    assert!(f("ipv4", "\"192.168.0.1\""));
    assert!(!f("ipv4", "\"087.10.10.1\""));
    assert!(f("ipv6", "\"2001:db8::1\""));
    assert!(!f("ipv6", "\"2001:db8:::1\""));
    assert!(f("uri", "\"https://example.com/a?b=c#d\""));
    assert!(!f("uri", "\"//missing scheme\""));
    assert!(f("uri-reference", "\"/relative/path\""));
    assert!(!f("uri-reference", "\"has space\""));
    assert!(f("uuid", "\"123e4567-e89b-12d3-a456-426614174000\""));
    assert!(!f("uuid", "\"123e4567e89b12d3a456426614174000\""));
    assert!(f("regex", "\"^a+b$\""));
    assert!(!f("regex", "\"(unclosed\""));
    assert!(f("json-pointer", "\"/a/b\""));
    assert!(f("json-pointer", "\"\""));
    assert!(!f("json-pointer", "\"a/b\""));
    assert!(f("duration", "\"P1Y2M3DT4H5M6.5S\""));
    assert!(f("duration", "\"PT10H\""));
    assert!(!f("duration", "\"P\""));
    assert!(!f("duration", "\"P1Y2M3DT\""));
    // unknown formats never assert
    assert!(f("x-custom", "\"anything\""));
}

// ---------------------------------------------------------------------------
// meta-data / content annotations
// ---------------------------------------------------------------------------

#[test]
fn meta_data_and_content_are_annotation_only() {
    let s = r##"{
        "title": "t", "description": "d", "default": 42,
        "deprecated": true, "readOnly": true, "writeOnly": true,
        "examples": [1, "x"], "contentMediaType": "application/json",
        "contentEncoding": "base64", "contentSchema": {"type": "string"},
        "type": "integer"
    }"##;
    assert!(valid(s, "5"));
    assert!(!valid(s, "\"s\""));
}

// ---------------------------------------------------------------------------
// engine behavior
// ---------------------------------------------------------------------------

#[test]
fn max_errors_early_exit() {
    let s = r##"{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"},"c":{"type":"string"},"d":{"type":"string"},"e":{"type":"string"}}}"##;
    let cfg = Config { max_errors: 2, ..Config::default() };
    bind_case!(cfg, s, r##"{"a":1,"b":2,"c":3,"d":4,"e":5}"##, _doc, schema, inst);
    let errors = schema.validate(inst);
    assert_eq!(errors.len(), 2, "capped at max_errors");

    bind_case!(Config::default(), s, r##"{"a":1,"b":2}"##, _doc2, schema2, inst2);
    assert_eq!(schema2.validate(inst2).len(), 2);
}

#[test]
fn validate_first_early_exit() {
    let s = r##"{"type":"object","properties":{"a":{"type":"string"},"b":{"type":"string"}}}"##;
    bind_case!(Config::default(), s, r##"{"a":1,"b":2}"##, _doc, schema, inst);
    assert!(schema.validate_first(inst).is_some());
    bind_case!(Config::default(), s, r##"{"a":"x","b":"y"}"##, _doc2, schema2, inst2);
    assert!(schema2.validate_first(inst2).is_none());
}

#[test]
fn too_deep_compile_error() {
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(too_deep_compile_error_inner)
        .expect("spawn");
    handle.join().expect("inner");
}

fn too_deep_compile_error_inner() {
    let mut schema = String::from("{\"type\":\"object\"}");
    for _ in 0..600 {
        schema = format!("{{\"properties\":{{\"a\":{}}}}}", schema);
    }
    let text = format!("{{\"S\": {schema}, \"I\": {{}}}}");
    let doc = parse(&text);
    match Compiler::new(Config::default()).compile(doc.root().get("S").expect("s")) {
        Err(CompileError::TooDeep { cap: 512 }) => {}
        _other => panic!("expected TooDeep {{cap:512}}"),
    }
    // a larger cap compiles the same schema
    let cfg = Config { max_depth: 1024, ..Config::default() };
    let doc2 = parse(&text);
    assert!(Compiler::new(cfg).compile(doc2.root().get("S").expect("s")).is_ok());
}

#[test]
fn error_paths_point_at_instance_and_keyword() {
    let s = r##"{
        "properties": {
            "name": {"type": "string", "maxLength": 3}
        }
    }"##;
    let e = invalid(s, r##"{"name":"toolong"}"##);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].instance_path.to_path(), "/name");
    assert_eq!(e[0].schema_path.to_path(), "/properties/name/maxLength");
    assert!(e[0].to_string().contains("maxLength"));
}

#[test]
fn schema_root_accessor_and_boolean_schemas() {
    bind_case!(Config::default(), r##"{"type":"integer"}"##, "3", _doc, s, inst);
    assert_eq!(s.root().get("type").and_then(|t| t.as_str()), Some("integer"));
    assert!(s.validate(inst).is_empty());
    assert!(valid("true", "anything"));
    assert!(!valid("false", "anything"));
    assert_eq!(invalid("false", "1")[0].message, "value matches `false` schema");
}
