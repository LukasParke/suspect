//! `suspect drift` unit tests: template matching, exchange parsing, and
//! the report computation over a small hand-built spec.

use suspect_cli::commands::drift::{Exchange, drift_report, exchange_from_line, from_method_url};
use suspect_ir::{IrOperation, IrParameter, IrSpec, Method, ParamIn};

fn query_param(name: &str) -> IrParameter {
    IrParameter {
        name: name.to_owned(),
        location: ParamIn::Query,
        required: false,
        schema: None,
    }
}

fn op(method: Method, path: &str, queries: &[&str]) -> IrOperation {
    IrOperation {
        id: None,
        method,
        path: path.to_owned(),
        summary: None,
        description: None,
        tags: Vec::new(),
        deprecated: false,
        parameters: queries.iter().map(|q| query_param(q)).collect(),
        body_schema: None,
        responses: Vec::new(),
    }
}

fn mini_spec() -> IrSpec {
    let operations = vec![
        op(Method::Get, "/library/sections/", &[]),
        op(Method::Get, "/hubs/search", &["query", "sectionId"]),
        op(Method::Post, "/playlists", &["title"]),
        op(Method::Get, "/library/metadata/{ratingKey}", &[]),
        op(Method::Get, "/playlists/{playlistId}/items", &[]),
        op(Method::Get, "/transcode/universal/start.{extension}", &[]),
    ];
    let mut spec = IrSpec::default();
    for op in operations {
        let idx = spec.operations.len() as u32;
        if let Some(id) = &op.id {
            spec.by_operation_id.insert(id.clone(), idx);
        }
        spec.by_method_path
            .insert((op.method, op.path.clone()), idx);
        spec.operations.push(op);
    }
    spec
}

fn ex(method: &str, url: &str) -> Exchange {
    from_method_url(method, url)
}

#[test]
fn url_parsing_handles_absolute_and_path_only() {
    let e = ex(
        "get",
        "http://localhost:32400/hubs/search?query=test&page=2",
    );
    assert_eq!(e.method, "GET");
    assert_eq!(e.path, "/hubs/search");
    assert_eq!(
        e.query,
        ["page", "query"].into_iter().map(str::to_owned).collect()
    );

    let e = ex("POST", "/playlists?title=CI");
    assert_eq!(e.method, "POST");
    assert_eq!(e.path, "/playlists");
    assert_eq!(e.query, ["title"].into_iter().map(str::to_owned).collect());
}

#[test]
fn journal_and_cassette_lines_both_parse() {
    let journal = serde_json::json!({
        "kind": "traffic", "method": "GET",
        "url": "http://x/identity", "status": 200,
        "verdict": {"pass": []}
    });
    let e = exchange_from_line(&journal).expect("journal line parses");
    assert_eq!(e.path, "/identity");

    let cassette = serde_json::json!({
        "id": 1, "method": "GET", "url": "http://x/identity",
        "status": 200, "request_headers": [], "response_headers": []
    });
    let e = exchange_from_line(&cassette).expect("cassette line parses");
    assert_eq!(e.path, "/identity");

    let meta = serde_json::json!({"kind": "meta", "msg": "started"});
    assert!(exchange_from_line(&meta).is_none());
    let header = serde_json::json!({"format": "suspect.cassette.v1"});
    assert!(exchange_from_line(&header).is_none());
}

#[test]
fn trailing_slash_and_templates_match() {
    let exchanges = vec![
        ex("GET", "http://x/library/sections"), // spec says `/library/sections/`
        ex("GET", "http://x/library/sections/"), // ...and its exact spelling
        ex("GET", "http://x/library/metadata/12345"), // `{ratingKey}` template
        ex("GET", "http://x/transcode/universal/start.m4s"), // embedded capture
        ex("POST", "http://x/playlists?title=CI"),
    ];
    let report = drift_report(&mini_spec(), &exchanges);
    assert!(
        report.missing_endpoints.is_empty(),
        "every documented spelling matches: {:?}",
        report.missing_endpoints
    );
}

#[test]
fn off_spec_paths_and_methods_report_as_missing() {
    let exchanges = vec![
        ex("GET", "http://x/not/in/spec"),
        ex("DELETE", "http://x/hubs/search"), // path known, method undeclared
    ];
    let report = drift_report(&mini_spec(), &exchanges);
    let missing: Vec<String> = report
        .missing_endpoints
        .iter()
        .map(|m| format!("{} {}", m.method, m.path))
        .collect();
    assert!(
        missing.contains(&"GET /not/in/spec".to_owned()),
        "{missing:?}"
    );
    assert!(
        missing.contains(&"DELETE /hubs/search".to_owned()),
        "an undeclared method on a known path is a gap: {missing:?}"
    );
}

#[test]
fn undeclared_query_params_report_per_operation() {
    let exchanges = vec![
        ex("GET", "http://x/hubs/search?query=test&count=10"),
        ex("POST", "http://x/playlists?title=CI&titleSort=zz"),
    ];
    let report = drift_report(&mini_spec(), &exchanges);
    let gaps: Vec<String> = report
        .query_param_gaps
        .iter()
        .map(|g| {
            format!(
                "{} {} {}",
                g.method,
                g.path,
                g.missing_query_params.join(",")
            )
        })
        .collect();
    assert!(
        gaps.contains(&"GET /hubs/search count".to_owned()),
        "undeclared params are named: {gaps:?}"
    );
    assert!(
        gaps.contains(&"POST /playlists titleSort".to_owned()),
        "declared params are not flagged: {gaps:?}"
    );
    assert_eq!(report.query_param_gaps.len(), 2);
}

#[test]
fn untested_operations_are_coverage_not_gaps() {
    let exchanges = vec![ex("GET", "http://x/identity")];
    let report = drift_report(&mini_spec(), &exchanges);
    // 6 declared ops, none matched: all untested, one missing (identity).
    assert_eq!(report.summary.untested_in_spec, 6);
    assert_eq!(report.summary.missing_from_spec, 1);
    assert_eq!(report.summary.endpoints_captured, 1);
    assert_eq!(report.summary.endpoints_in_spec, 6);
}
