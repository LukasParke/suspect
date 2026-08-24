use std::collections::HashMap;
use std::sync::LazyLock;

use super::*;
use suspect_low::Pointer;

fn ptr(tokens: &[&str]) -> Pointer {
    Pointer::from_tokens(tokens.iter().map(|t| (*t).into()).collect())
}

// ---------------------------------------------------------------------------
// Parse table: every variant parses to the contracted enum shape.
// ---------------------------------------------------------------------------

#[test]
fn parse_grammar_table() {
    let cases: &[(&str, Rex)] = &[
        ("$method", Rex::Method),
        ("$statusCode", Rex::StatusCode),
        (
            "$request.header.X-Request-Id",
            Rex::Request {
                part: Part::Header("X-Request-Id".to_owned()),
                pointer: Pointer::root(),
            },
        ),
        (
            "$request.query.limit",
            Rex::Request {
                part: Part::Query("limit".to_owned()),
                pointer: Pointer::root(),
            },
        ),
        (
            "$request.path.petId",
            Rex::Request {
                part: Part::Path("petId".to_owned()),
                pointer: Pointer::root(),
            },
        ),
        (
            "$request.body",
            Rex::Request {
                part: Part::Body,
                pointer: Pointer::root(),
            },
        ),
        // Empty fragment == root document.
        (
            "$request.body#",
            Rex::Request {
                part: Part::Body,
                pointer: Pointer::root(),
            },
        ),
        // ~1 unescapes to `/` inside the final token.
        (
            "$request.body#/a/b~1c",
            Rex::Request {
                part: Part::Body,
                pointer: ptr(&["a", "b/c"]),
            },
        ),
        // ~0 unescapes to `~`.
        (
            "$response.body#/a~0b",
            Rex::Response {
                part: Part::Body,
                pointer: ptr(&["a~b"]),
            },
        ),
        // RFC 6901 §6: %2F decodes to `/` before pointer evaluation, so it
        // splits; ~-unescaping then applies to the decoded text only.
        (
            "$response.body#/a%20b/c%2Fd",
            Rex::Response {
                part: Part::Body,
                pointer: ptr(&["a b", "c", "d"]),
            },
        ),
        (
            "$response.header.Content-Type",
            Rex::Response {
                part: Part::Header("Content-Type".to_owned()),
                pointer: Pointer::root(),
            },
        ),
        (
            "$response.body#/x/y",
            Rex::Response {
                part: Part::Body,
                pointer: ptr(&["x", "y"]),
            },
        ),
        (
            "$inputs.name",
            Rex::Inputs {
                key: "name".to_owned(),
            },
        ),
        // Dots are allowed inside plain keys.
        (
            "$inputs.user.name",
            Rex::Inputs {
                key: "user.name".to_owned(),
            },
        ),
        (
            "$inputs#/user/tags/0",
            Rex::Inputs {
                key: "#/user/tags/0".to_owned(),
            },
        ),
        (
            "$steps.createUser.outputs.id",
            Rex::Steps {
                step: "createUser".to_owned(),
                outputs_key: "id".to_owned(),
            },
        ),
        (
            "$steps.createUser.outputs#/links/self",
            Rex::Steps {
                step: "createUser".to_owned(),
                outputs_key: "#/links/self".to_owned(),
            },
        ),
        (
            "$sourceDescriptions.petstore#/paths/~1pets/get",
            Rex::SourceDescriptions {
                name: "petstore".to_owned(),
                pointer: ptr(&["paths", "/pets", "get"]),
            },
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(&parse_rex(input).unwrap(), expected, "input: {input}");
    }
}

#[test]
fn text_passthrough_parse() {
    assert_eq!(parse_rex("hello").unwrap(), Rex::Text("hello".to_owned()));
    assert_eq!(
        parse_rex("no dollar here").unwrap(),
        Rex::Text("no dollar here".to_owned())
    );
}

#[test]
fn parse_errors() {
    let bad = [
        // `$url` has no Rex representation.
        "$url",
        // Unknown expression / top-level segment.
        "$foo.bar",
        "$statuscode",
        // Lone `$`.
        "$",
        // Trailing garbage after an exact expression.
        "$method ",
        // Invalid location after $request.
        "$request.",
        "$request.cookie.sid",
        // Empty names.
        "$request.header.",
        "$inputs.",
        "$steps.a.outputs.",
        // Missing pieces.
        "$inputs",
        "$steps.a.b.c",
        "$sourceDescriptions.petstore",
        // Invalid pointer characters / escapes.
        "$request.body#/a[",
        "$response.body#/a b",
        "$response.body#/nested#/again",
        "$request.body#/bad~2escape",
    ];
    for input in bad {
        let err = parse_rex(input).expect_err(input);
        let msg = err.to_string();
        assert!(msg.contains(input), "error must quote the input: {msg}");
        fn assert_error_impl<E: std::error::Error>(_: &E) {}
        assert_error_impl(&err);
    }
}

#[test]
fn error_display_mentions_position_and_reason() {
    let msg = parse_rex("$request.header.").unwrap_err().to_string();
    assert!(msg.contains("offset"), "position in message: {msg}");
    assert!(msg.contains("`$request.header.`"), "reason quotes input");
}

// ---------------------------------------------------------------------------
// Evaluation table.
// ---------------------------------------------------------------------------

static REQ_HEADERS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    vec![
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("X-Request-Id".to_owned(), "req-1".to_owned()),
    ]
});
static RESP_HEADERS: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("X-RateLimit-Remaining".to_owned(), "99".to_owned()),
    ]
});
static INPUTS: LazyLock<serde_json::Map<String, serde_json::Value>> = LazyLock::new(|| {
    serde_json::from_str(r#"{"user": {"name": "ada", "tags": ["admin"]}, "mode": "live"}"#).unwrap()
});
static STEPS_OUTPUTS: LazyLock<serde_json::Map<String, serde_json::Value>> = LazyLock::new(|| {
    serde_json::from_str(r#"{"createUser": {"id": 7, "links": {"self": "/users/7"}}}"#).unwrap()
});
static DESCRIPTIONS: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    HashMap::from([(
        "petstore".to_owned(),
        r#"{"paths": {"/pets": {"get": {"summary": "list"}}}}"#.to_owned(),
    )])
});

const REQ_BODY: &str = r#"{"pet": {"name": {"first": "Rex"}, "tags/a": ["dog"]}}"#;
const RESP_BODY: &str = r#"{"created": true, "id": 42}"#;

fn fixture_ctx<'a>(request_body: &'a str, response_body: &'a str) -> RexCtx<'a> {
    RexCtx::default()
        .method("POST")
        .status(201)
        .request_headers(REQ_HEADERS.as_slice())
        .response_headers(RESP_HEADERS.as_slice())
        .request_body(request_body)
        .response_body(response_body)
        .inputs(&INPUTS)
        .steps_outputs(&STEPS_OUTPUTS)
        .source_descriptions(&DESCRIPTIONS)
}

#[test]
fn eval_method_and_status() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    assert_eq!(
        eval_rex(&parse_rex("$method").unwrap(), &ctx),
        Some(serde_json::json!("POST"))
    );
    assert_eq!(
        eval_rex(&parse_rex("$statusCode").unwrap(), &ctx),
        Some(serde_json::json!(201))
    );
}

#[test]
fn eval_headers_case_insensitive() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$request.header.X-REQUEST-id").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("req-1")));
    let rex = parse_rex("$response.header.CONTENT-TYPE").unwrap();
    assert_eq!(
        eval_rex(&rex, &ctx),
        Some(serde_json::json!("application/json"))
    );
}

#[test]
fn eval_missing_header_is_none() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$response.header.Set-Cookie").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), None);
}

#[test]
fn eval_body_pointers_nested_and_escapes() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$request.body#/pet/name/first").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("Rex")));
    // ~1 addresses a literal `/` inside a JSON key.
    let rex = parse_rex("$request.body#/pet/tags~1a/0").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("dog")));
    // Root pointer returns the whole document.
    let rex = parse_rex("$response.body#").unwrap();
    assert_eq!(
        eval_rex(&rex, &ctx),
        Some(serde_json::json!({"created": true, "id": 42}))
    );
}

#[test]
fn eval_body_pointer_misses_are_none() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    for expr in [
        "$request.body#/missing/deeply",
        "$request.body#/pet/name/first/too/far",
        // Array index into an object member.
        "$response.body#/id/0",
    ] {
        assert_eq!(eval_rex(&parse_rex(expr).unwrap(), &ctx), None, "{expr}");
    }
}

#[test]
fn eval_non_json_body_is_none() {
    let ctx = fixture_ctx("not json at all {", RESP_BODY);
    let rex = parse_rex("$request.body#/a").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), None);
}

#[test]
fn eval_query_and_path_parts_have_no_context_source_yet() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    assert_eq!(
        eval_rex(&parse_rex("$request.query.limit").unwrap(), &ctx),
        None
    );
    assert_eq!(
        eval_rex(&parse_rex("$request.path.petId").unwrap(), &ctx),
        None
    );
}

#[test]
fn eval_inputs_direct_and_pointer() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$inputs.mode").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("live")));
    let rex = parse_rex("$inputs#/user/tags/0").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("admin")));
    assert_eq!(eval_rex(&parse_rex("$inputs.nothing").unwrap(), &ctx), None);
}

#[test]
fn eval_steps_outputs() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$steps.createUser.outputs.id").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!(7)));
    let rex = parse_rex("$steps.createUser.outputs#/links/self").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("/users/7")));
    assert_eq!(
        eval_rex(&parse_rex("$steps.missing.outputs.x").unwrap(), &ctx),
        None
    );
    assert_eq!(
        eval_rex(
            &parse_rex("$steps.createUser.outputs.missing").unwrap(),
            &ctx
        ),
        None
    );
}

#[test]
fn eval_source_descriptions() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$sourceDescriptions.petstore#/paths/~1pets/get/summary").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), Some(serde_json::json!("list")));
    assert_eq!(
        eval_rex(&parse_rex("$sourceDescriptions.missing#/a").unwrap(), &ctx),
        None
    );
}

#[test]
fn eval_non_json_source_description_is_none() {
    static BROKEN: LazyLock<HashMap<String, String>> =
        LazyLock::new(|| HashMap::from([("broken".to_owned(), "{not json".to_owned())]));
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY).source_descriptions(&BROKEN);
    let rex = parse_rex("$sourceDescriptions.broken#/a").unwrap();
    assert_eq!(eval_rex(&rex, &ctx), None);
}

#[test]
fn eval_text_passthrough() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    assert_eq!(
        eval_rex(&Rex::Text("plain".to_owned()), &ctx),
        Some(serde_json::json!("plain"))
    );
}

#[test]
fn eval_is_deterministic() {
    let ctx = fixture_ctx(REQ_BODY, RESP_BODY);
    let rex = parse_rex("$request.body#/pet/name/first").unwrap();
    let first = eval_rex(&rex, &ctx);
    for _ in 0..5 {
        assert_eq!(eval_rex(&rex, &ctx), first);
    }
}

#[test]
fn percent_decoding_precedes_pointer_unescaping() {
    // RFC 6901 §6 order: percent-decode the whole fragment first, then
    // ~-unescape. `%2F` becomes a separator; `~1` inside decoded text is
    // still unescaped.
    let rex = parse_rex("$response.body#/x~1y%2Fz").unwrap();
    // Decoded `%2F` splits the pointer; the surviving `~1` then unescapes
    // inside its own token.
    assert_eq!(
        rex,
        Rex::Response {
            part: Part::Body,
            pointer: ptr(&["x/y", "z"]),
        }
    );
}

#[test]
fn array_indices_reject_leading_zeros_and_signs() {
    let body = r#"{"arr": ["a", "b"]}"#;
    let ctx = fixture_ctx(body, body);
    // Well-formed index resolves.
    assert_eq!(
        eval_rex(&parse_rex("$request.body#/arr/1").unwrap(), &ctx),
        Some(serde_json::json!("b"))
    );
    // Non-canonical spellings must not address array elements.
    for frag in ["/arr/01", "/arr/+1", "/arr/-0"] {
        let expr = format!("$request.body#{frag}");
        let rex = parse_rex(&expr).unwrap_or_else(|e| panic!("{expr}: {e}"));
        assert_eq!(eval_rex(&rex, &ctx), None, "{expr}");
    }
}
