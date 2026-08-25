//! Handler-to-spec reverse engineering.
//!
//! Parses server framework route registrations (Rust axum/actix, TypeScript
//! Express, Go net/http & Gin) and cross-references them against the spec
//! to detect undocumented endpoints and parameter mismatches.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

/// One route extracted from server source code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ExtractedRoute {
    /// HTTP method (uppercased).
    pub method: String,
    /// Path pattern as registered (framework-specific syntax normalized).
    pub path: String,
    /// Source file.
    pub file: String,
    /// Line number (1-based).
    pub line: u32,
    /// Framework detected.
    pub framework: String,
}

/// One mismatch between implementation and spec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mismatch {
    /// Kind: `undocumented_endpoint` | `spec_only_endpoint` |
    /// `method_mismatch`.
    pub kind: String,
    /// The route involved.
    pub route: String,
    /// Human description.
    pub message: String,
    /// Suggested spec fragment to add (for undocumented endpoints).
    pub spec_fragment: Option<String>,
}

/// The full cross-reference report.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReverseReport {
    /// All routes extracted from server code.
    pub extracted: Vec<ExtractedRoute>,
    /// Endpoints implemented but missing from the spec.
    pub undocumented: Vec<Mismatch>,
    /// Endpoints in the spec but not implemented.
    pub spec_only: Vec<Mismatch>,
    /// Same path, different method sets.
    pub method_mismatches: Vec<Mismatch>,
}

/// Framework route patterns, compiled once.
static AXUM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\.route\(\s*"([^"]+)"\s*,\s*(get|post|put|delete|patch|head|options)\("#)
        .expect("valid regex")
});
static ACTIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"#\[(get|post|put|delete|patch|head|options)\("([^"]+)"\)\]"#)
        .expect("valid regex")
});
static EXPRESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\b(?:app|router|api)\.(get|post|put|delete|patch|head|options)\s*\(\s*['"`]([^'"`]+)['"`]"#,
    )
    .expect("valid regex")
});
static GO_HTTP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\.HandleFunc\(\s*"([^"]+)"\s*,"#).expect("valid regex"));
static GIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\.\b(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS|Any)\(\s*"([^"]+)""#)
        .expect("valid regex")
});
static PATH_PARAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[:*]([a-zA-Z_][a-zA-Z0-9_]*)").expect("valid regex"));

/// Extracts routes from a source file by framework pattern matching.
#[must_use]
pub fn extract_routes(source: &str, file: &str) -> Vec<ExtractedRoute> {
    let mut routes = Vec::new();

    // --- Rust axum: .route("/path", get(handler)) or routing DSL ---
    let axum = &*AXUM;
    for cap in axum.captures_iter(source) {
        let line = line_of(source, cap.get(0).map(|m| m.start()).unwrap_or(0));
        routes.push(ExtractedRoute {
            method: cap[2].to_uppercase(),
            path: normalize_path(&cap[1]),
            file: file.to_owned(),
            line,
            framework: "axum".to_owned(),
        });
    }

    // --- Rust actix: #[get("/path")] attribute macros ---
    let actix = &*ACTIX;
    for cap in actix.captures_iter(source) {
        let line = line_of(source, cap.get(0).map(|m| m.start()).unwrap_or(0));
        routes.push(ExtractedRoute {
            method: cap[1].to_uppercase(),
            path: normalize_path(&cap[2]),
            file: file.to_owned(),
            line,
            framework: "actix".to_owned(),
        });
    }

    // --- TypeScript Express: app.get('/path', ...) / router.post(...) ---
    let express = &*EXPRESS;
    for cap in express.captures_iter(source) {
        let line = line_of(source, cap.get(0).map(|m| m.start()).unwrap_or(0));
        routes.push(ExtractedRoute {
            method: cap[1].to_uppercase(),
            path: normalize_path(&cap[2]),
            file: file.to_owned(),
            line,
            framework: "express".to_owned(),
        });
    }

    // --- Go net/http: r.HandleFunc("/path", handler).Methods("GET") ---
    let go_http = &*GO_HTTP;
    for cap in go_http.captures_iter(source) {
        let line = line_of(source, cap.get(0).map(|m| m.start()).unwrap_or(0));
        // net/http without Methods() handles all methods; record as ANY
        routes.push(ExtractedRoute {
            method: "ANY".to_owned(),
            path: normalize_path(&cap[1]),
            file: file.to_owned(),
            line,
            framework: "go-http".to_owned(),
        });
    }

    // --- Go Gin: r.GET("/path", handler) ---
    let gin = &*GIN;
    for cap in gin.captures_iter(source) {
        let line = line_of(source, cap.get(0).map(|m| m.start()).unwrap_or(0));
        let method = if &cap[1] == "Any" {
            "ANY".to_owned()
        } else {
            cap[1].to_string()
        };
        routes.push(ExtractedRoute {
            method,
            path: normalize_path(&cap[2]),
            file: file.to_owned(),
            line,
            framework: "gin".to_owned(),
        });
    }

    routes
}

/// Walks a directory tree, extracting routes from supported source files.
#[must_use]
pub fn extract_from_tree(root: &Path) -> Vec<ExtractedRoute> {
    let mut routes = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return routes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "node_modules" | ".git" | "vendor" | "dist") {
                continue;
            }
            routes.extend(extract_from_tree(&path));
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && matches!(ext, "rs" | "ts" | "tsx" | "js" | "go")
            && let Ok(source) = std::fs::read_to_string(&path)
        {
            let file = path.to_string_lossy().to_string();
            routes.extend(extract_routes(&source, &file));
        }
    }
    routes
}

/// Cross-references extracted routes against the spec IR.
#[must_use]
pub fn cross_reference(spec: &suspect_ir::IrSpec, routes: &[ExtractedRoute]) -> ReverseReport {
    let mut report = ReverseReport {
        extracted: routes.to_vec(),
        ..ReverseReport::default()
    };

    // Normalize both sides: "METHOD /path"
    let mut impl_set: BTreeMap<(String, String), &ExtractedRoute> = BTreeMap::new();
    for r in routes {
        impl_set.insert((r.method.clone(), normalize_oas(r.path.clone())), r);
    }

    let mut spec_set: BTreeMap<(String, String), ()> = BTreeMap::new();
    for op in &spec.operations {
        spec_set.insert(
            (
                op.method.as_str().to_uppercase(),
                normalize_oas(op.path.clone()),
            ),
            (),
        );
    }

    // Implemented but undocumented
    for ((method, path), route) in &impl_set {
        let spec_has = spec_set.contains_key(&("ANY".to_owned(), path.clone()))
            || spec_set.contains_key(&(method.clone(), path.clone()));
        if !spec_has && method != "ANY" {
            report.undocumented.push(Mismatch {
                kind: "undocumented_endpoint".to_owned(),
                route: format!("{method} {path}"),
                message: format!(
                    "Endpoint implemented in {} at {}:{} but missing from the spec",
                    route.framework, route.file, route.line
                ),
                spec_fragment: Some(spec_fragment_for(method, path)),
            });
        }
    }

    // Documented but not implemented
    for (method, path) in spec_set.keys() {
        let impl_has = impl_set
            .keys()
            .any(|(m, p)| (p == path && (m == method || m == "ANY")) || (m == "ANY" && p == path));
        if !impl_has {
            report.spec_only.push(Mismatch {
                kind: "spec_only_endpoint".to_owned(),
                route: format!("{method} {path}"),
                message: "Endpoint documented in the spec but not found in server code".to_string(),
                spec_fragment: None,
            });
        }
    }

    // Method mismatches: same path, disjoint methods
    let mut impl_by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (m, p) in impl_set.keys() {
        impl_by_path.entry(p.clone()).or_default().insert(m.clone());
    }
    let mut spec_by_path: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (m, p) in spec_set.keys() {
        spec_by_path.entry(p.clone()).or_default().insert(m.clone());
    }
    for (path, impl_methods) in &impl_by_path {
        if let Some(spec_methods) = spec_by_path.get(path)
            && impl_methods.is_disjoint(spec_methods)
        {
            report.method_mismatches.push(Mismatch {
                kind: "method_mismatch".to_owned(),
                route: path.clone(),
                message: format!(
                    "Path `{path}` implements {impl_methods:?} but spec declares {spec_methods:?}"
                ),
                spec_fragment: None,
            });
        }
    }

    report
}

/// Normalizes framework path syntax to OpenAPI `{param}` style.
fn normalize_path(p: &str) -> String {
    // axum: :param → {param}; express: :param → {param}
    PATH_PARAM.replace_all(p, "{$1}").to_string()
}

/// Normalizes an OAS path for comparison (trailing slashes, etc.).
fn normalize_oas(p: String) -> String {
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn line_of(source: &str, byte_offset: usize) -> u32 {
    source[..byte_offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count() as u32
        + 1
}

fn spec_fragment_for(method: &str, path: &str) -> String {
    format!(
        "  /{}:\n    {}:\n      summary: Undocumented endpoint (reverse-engineered)\n      responses:\n        '200':\n          description: OK",
        path.trim_start_matches('/'),
        method.to_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_axum_express_and_gin_routes() {
        let rust = r#"app.route("/pets", get(list)).route("/pets/{id}", delete(del));"#;
        let routes = extract_routes(rust, "main.rs");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].method, "GET");
        assert_eq!(routes[0].framework, "axum");

        let ts = "app.get('/users', handler); router.post('/users', create);";
        let routes = extract_routes(ts, "app.ts");
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].framework, "express");

        let go = r#"r.GET("/ping", pingHandler)"#;
        let routes = extract_routes(go, "main.go");
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].method, "GET");
    }

    #[test]
    fn normalizes_framework_params_to_oas() {
        let routes = extract_routes(r#"app.get("/users/:userId", h);"#, "a.ts");
        assert_eq!(routes[0].path, "/users/{userId}");
    }

    #[test]
    fn cross_reference_finds_undocumented_and_spec_only() {
        let spec = suspect_ir::IrSpec {
            operations: vec![suspect_ir::IrOperation {
                id: Some("documentedOnly".to_owned()),
                method: suspect_ir::Method::Get,
                path: "/only-in-spec".to_owned(),
                summary: None,
                description: None,
                tags: Vec::new(),
                deprecated: false,
                parameters: Vec::new(),
                body_schema: None,
                responses: Vec::new(),
            }],
            ..suspect_ir::IrSpec::default()
        };
        let routes = vec![ExtractedRoute {
            method: "GET".to_owned(),
            path: "/only-in-code".to_owned(),
            file: "main.rs".to_owned(),
            line: 1,
            framework: "axum".to_owned(),
        }];
        let report = cross_reference(&spec, &routes);
        assert_eq!(report.undocumented.len(), 1);
        assert_eq!(report.undocumented[0].route, "GET /only-in-code");
        assert!(report.undocumented[0].spec_fragment.is_some());
        assert_eq!(report.spec_only.len(), 1);
        assert_eq!(report.spec_only[0].route, "GET /only-in-spec");
    }
}
