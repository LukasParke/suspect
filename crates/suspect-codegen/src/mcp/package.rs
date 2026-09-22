//! Pinned npm package metadata for the single emitted application package.
//!
//! The official MCP server SDK is an application-only dependency, pinned to a
//! registry-verified release together with the exact transitive versions and
//! integrity digests its own reviewed lockfile resolved. The embedded canonical
//! SDK is a source directory of this same package, so it is never a dependency
//! and never keeps its own package metadata or lockfile.
//!
//! The official MCP client package is deliberately absent: it is an acceptance
//! testing tool, and a server that shipped it would carry a dependency it never
//! uses.
use serde_json::{Map, Value, json};

use super::{MCP_SERVER_VERSION, NODE_TYPES_VERSION, ServerPlan, TYPESCRIPT_VERSION};
use crate::OutFile;

/// Paths `npm pack` keeps, so a packed tarball is still a buildable source tree
/// and still carries the reviewable surface manifest this manifest points at.
const PACKED: &[&str] = &[
    "application-surface.json",
    "dist",
    "server",
    "typescript",
    "README.md",
    "tsconfig.json",
];

/// The compiled entry point the executable and `bin` entry both name.
pub(super) const ENTRY: &str = "./dist/server/main.js";

pub(super) fn artifacts(plan: &ServerPlan) -> Vec<OutFile> {
    let config = &plan.config;
    let engines = json!({"node": format!(">={}", config.node_minimum_major)});
    let dependencies = json!({"@modelcontextprotocol/server": MCP_SERVER_VERSION});
    let development = json!({
        "@types/node": NODE_TYPES_VERSION,
        "typescript": TYPESCRIPT_VERSION,
    });
    let mut bin = Map::new();
    bin.insert(config.bin_name.clone(), Value::from(ENTRY));
    let bin = Value::Object(bin);

    let package = json!({
        "name": config.package_name,
        "version": config.version,
        "private": true,
        "description": format!(
            "Generated stdio Model Context Protocol server for {}",
            config.server_name
        ),
        "type": "module",
        "bin": bin,
        "files": PACKED,
        "engines": engines,
        "packageManager": format!("npm@{}", config.npm_version),
        "scripts": {"build": "tsc --project tsconfig.json"},
        "dependencies": dependencies,
        "devDependencies": development,
        "suspect": {
            "kind": "mcp-server",
            "transport": "stdio",
            "surface": "application-surface.json",
            "sdk": "typescript/http-manifest.json",
            "toolchain": {
                "node": config.node_version,
                "npm": config.npm_version,
                "typescript": TYPESCRIPT_VERSION,
            },
        },
    });

    // Reuse the reviewed registry resolution and integrity of the exact pinned
    // dependency set. Only the root entry is rewritten, so every resolved
    // version and digest stays byte-identical to the reviewed lockfile.
    let mut lock: Value = serde_json::from_str(include_str!("dependencies.lock.json"))
        .expect("reviewed lock is JSON");
    assert_eq!(
        lock["packages"]["node_modules/@modelcontextprotocol/server"]["version"],
        MCP_SERVER_VERSION
    );
    assert_eq!(
        lock["packages"]["node_modules/typescript"]["version"],
        TYPESCRIPT_VERSION
    );
    assert_eq!(
        lock["packages"]["node_modules/@types/node"]["version"],
        NODE_TYPES_VERSION
    );
    lock["name"] = Value::from(config.package_name.clone());
    lock["version"] = Value::from(config.version.clone());
    lock["packages"][""] = json!({
        "name": config.package_name,
        "version": config.version,
        "dependencies": dependencies,
        "devDependencies": development,
        "engines": engines,
        "bin": bin,
    });

    // The entry point is compiled with the same options the canonical SDK is
    // verified under, so the embedded sources build exactly as they do inside
    // their own package. Only the entry point is listed: the compiler follows
    // its imports, so the SDK's reference documentation and its unreferenced
    // modules are never compiled into this application.
    let build = json!({
        "compilerOptions": {
            "target": "ES2022", "module": "NodeNext", "moduleResolution": "NodeNext",
            "strict": true, "exactOptionalPropertyTypes": true, "noUncheckedIndexedAccess": true,
            "declaration": true, "declarationMap": true, "sourceMap": true,
            "rootDir": ".", "outDir": "dist", "noEmitOnError": true,
            "forceConsistentCasingInFileNames": true,
        },
        "files": ["server/main.ts"],
    });

    vec![
        OutFile {
            path: "package.json".into(),
            content: text(&package),
        },
        OutFile {
            path: "package-lock.json".into(),
            content: text(&lock),
        },
        OutFile {
            path: "tsconfig.json".into(),
            content: text(&build),
        },
    ]
}

fn text(value: &Value) -> String {
    let mut out = serde_json::to_string_pretty(value).expect("package metadata is JSON");
    out.push('\n');
    out
}
