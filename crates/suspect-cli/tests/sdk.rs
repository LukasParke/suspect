//! Canonical generation through the public CLI, including its read-only CI mode.

use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args([
            "codegen",
            "api.json",
            "--profile",
            "typescript-http",
            "--package-name",
            "@fixture/sdk",
            "--package-version",
            "0.0.0",
            "--out",
            "output",
            "--format",
            "json",
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn fixture() -> Value {
    json!({
        "openapi":"3.1.0", "info":{"title":"SDK","version":"1"},
        "servers":[{"url":"https://example.com/api/v1"}],
        "security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
        "paths":{"/credits":{"get":{"operationId":"getCredits","responses":{
            "200":{"description":"Balance","content":{"application/json":{"schema":{
                "type":"object","properties":{"balance":{"type":"number"}},"required":["balance"]
            }}}}
        }}}}
    })
}

fn report(output: &Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn canonical_sdk_generation_and_check_share_the_owned_artifact_contract() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api.json"), fixture().to_string()).unwrap();
    // Unrelated invalid neighbors are not source inputs.
    std::fs::write(dir.path().join("unrelated.json"), "{broken").unwrap();
    let check = report(&run(dir.path(), &["--check"]), 1);
    assert_eq!(check["status"], "drift");
    assert!(!dir.path().join("output").exists());
    let written = report(&run(dir.path(), &[]), 0);
    assert_eq!(written["status"], "generated");
    assert_eq!(written["releaseReady"], false);
    assert_eq!(written["operations"][0]["operationId"], "getCredits");
    let root = dir.path().join("output/typescript");
    let package: Value =
        serde_json::from_slice(&std::fs::read(root.join("package.json")).unwrap()).unwrap();
    assert_eq!(package["name"], "@fixture/sdk");
    assert_eq!(package["private"], true);
    assert!(root.join("http-manifest.json").exists());
    assert!(root.join("model-codecs.ts").exists());
    assert!(root.join("http.md").exists());
    assert_eq!(
        report(&run(dir.path(), &["--check"]), 0)["status"],
        "current"
    );
    std::fs::write(root.join("operations.ts"), "user edit").unwrap();
    assert_eq!(
        report(&run(dir.path(), &["--check"]), 1)["status"],
        "conflict"
    );
    assert_eq!(report(&run(dir.path(), &[]), 1)["status"], "conflict");
    assert_eq!(
        std::fs::read_to_string(root.join("operations.ts")).unwrap(),
        "user edit"
    );
}

#[test]
fn operation_selection_is_exact_and_rejections_preserve_existing_output() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    let mut spec = fixture();
    spec["paths"]["/unsupported"] = json!({"get":{"operationId":"unsupported","security":[{"missing":[]}],"responses":{"204":{"description":"empty"}}}});
    std::fs::write(&path, serde_json::to_string_pretty(&spec).unwrap()).unwrap();
    let selected = report(&run(dir.path(), &["--operation-id", "getCredits"]), 0);
    assert_eq!(selected["operations"].as_array().unwrap().len(), 1);
    let artifact = dir.path().join("output/typescript/operations.ts");
    let original = std::fs::read(&artifact).unwrap();
    let all = report(&run(dir.path(), &[]), 1);
    assert_eq!(all["status"], "failed");
    assert!(
        all["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["code"] == "http-security-scheme-unresolved"
                && d["pointer"] == "/paths/~1unsupported/get/security/0/missing"
                && d["line"].as_u64().is_some_and(|n| n > 1))
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), original);
    let absent = report(&run(dir.path(), &["--operation-id", "getCredit"]), 1);
    assert_eq!(absent["diagnostics"][0]["code"], "sdk-operation-not-found");
    spec["paths"]["/unsupported"]["get"]["operationId"] = json!("getCredits");
    std::fs::write(&path, spec.to_string()).unwrap();
    let ambiguous = report(&run(dir.path(), &["--operation-id", "getCredits"]), 1);
    assert_eq!(ambiguous["diagnostics"].as_array().unwrap().len(), 2);
    assert!(
        ambiguous["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["code"] == "sdk-operation-ambiguous")
    );
    assert_eq!(std::fs::read(&artifact).unwrap(), original);
}

fn checked(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
#[ignore = "requires all tracked OpenRouter inputs and the pinned native TS/npm/TypeDoc toolchain"]
fn tracked_openrouter_cli_package_builds_installs_calls_and_documents() {
    let source_root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let spec = Path::new(&source_root).join("projects/docs/openapi/openapi.yaml");
    assert!(
        spec.is_file(),
        "tracked primary OpenRouter spec is required"
    );
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    assert_eq!(
        String::from_utf8(checked(Command::new(&node).arg("--version")).stdout)
            .unwrap()
            .trim(),
        "v22.23.1"
    );
    assert_eq!(
        String::from_utf8(checked(Command::new("npm").arg("--version")).stdout)
            .unwrap()
            .trim(),
        "10.9.8"
    );
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("generated");
    let mut command = Command::new(env!("CARGO_BIN_EXE_suspect"));
    command
        .arg("codegen")
        .arg(&spec)
        .args([
            "--profile",
            "typescript-http",
            "--operation-id",
            "getCredits",
            "--operation-id",
            "createKeys",
            "--operation-id",
            "updateKeys",
            "--package-name",
            "@fixture/openrouter-sdk",
            "--package-version",
            "0.0.0",
            "--format",
            "json",
            "--out",
        ])
        .arg(&out);
    let generated = report(&command.output().unwrap(), 0);
    assert_eq!(generated["operations"].as_array().unwrap().len(), 3);
    let package = out.join("typescript");
    checked(Command::new("npm").current_dir(&package).args([
        "ci",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
    ]));
    checked(
        Command::new("npm")
            .current_dir(&package)
            .args(["run", "build"]),
    );
    let packed: Value = serde_json::from_slice(
        &checked(Command::new("npm").current_dir(&package).args([
            "pack",
            "--ignore-scripts",
            "--json",
        ]))
        .stdout,
    )
    .unwrap();
    let tarball = package.join(packed[0]["filename"].as_str().unwrap());
    let consumer = dir.path().join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        r#"{"name":"sdk-consumer","version":"0.0.0","private":true,"type":"module"}"#,
    )
    .unwrap();
    checked(
        Command::new("npm")
            .current_dir(&consumer)
            .args(["install", "--ignore-scripts", "--no-audit", "--no-fund"])
            .arg(&tarball),
    );
    std::fs::write(consumer.join("consumer.ts"), r#"
import { createClient, JsonNumber } from '@fixture/openrouter-sdk';
export async function run(serverURL: string): Promise<void> {
    const client = createClient({ auth: { apiKey: 'fixture-token' }, serverURL });
    const result = await client.getCredits({});
    const status: 200 = result.status;
    const media: 'application/json' = result.contentType;
    const credits: JsonNumber = result.data.data.total_credits;
    if (status !== 200 || media !== 'application/json' || credits.toString() !== '1.0000000000000000001') throw new Error('exact credits response was changed');
}
"#).unwrap();
    checked(
        Command::new(&node)
            .current_dir(&consumer)
            .arg(package.join("node_modules/typescript/bin/tsc"))
            .args([
                "--strict",
                "--exactOptionalPropertyTypes",
                "--noUncheckedIndexedAccess",
                "--target",
                "ES2022",
                "--module",
                "NodeNext",
                "--moduleResolution",
                "NodeNext",
                "consumer.ts",
            ]),
    );
    std::fs::write(
        consumer.join("run.mjs"),
        r#"
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { run } from './consumer.js';
let seen = 0;
let requestFailure;
const server = createServer((request, response) => {
    try {
        assert.equal(request.url, '/api/v1/credits');
        assert.equal(request.method, 'GET');
        assert.equal(request.headers.authorization, 'Bearer fixture-token');
        seen++;
        response.writeHead(200, {'content-type': 'application/json'});
        response.end('{"data":{"total_credits":1.0000000000000000001,"total_usage":0}}');
    } catch (error) { requestFailure = error; response.writeHead(500); response.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
try {
    await run(`http://127.0.0.1:${server.address().port}/api/v1`);
    if (requestFailure) throw requestFailure;
    assert.equal(seen, 1);
} finally { await new Promise(resolve => server.close(resolve)); }
"#,
    )
    .unwrap();
    checked(Command::new(&node).current_dir(&consumer).arg("run.mjs"));
    let docs_tool = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suspect-codegen/tools/typescript-docs/build.mjs");
    checked(Command::new(&node).arg(&docs_tool).arg(&package));
    assert!(package.join("docs/html/index.html").is_file());
    command.arg("--check");
    assert_eq!(report(&command.output().unwrap(), 0)["status"], "current");
}

#[test]
fn incomplete_or_unknown_generation_options_fail_before_writes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("api.json"), fixture().to_string()).unwrap();
    let invalid: &[&[&str]] = &[
        &["codegen", "--targets", "ts"],
        &["codegen", "--zod"],
        &[
            "codegen",
            "--package-name",
            "@fixture/sdk",
            "--package-version",
            "0.0.0",
        ],
        &["codegen", "--profile", "typescript-http"],
        &[
            "sdk",
            "--profile",
            "typescript-http",
            "--package-name",
            "@fixture/sdk",
            "--package-version",
            "0.0.0",
        ],
        &["gen", "--preset", "unknown"],
    ];
    for args in invalid {
        let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
            .current_dir(dir.path())
            .args(*args)
            .args(["api.json", "--out", "output"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "unexpected success for {args:?}");
        assert!(
            !dir.path().join("output").exists(),
            "wrote output for {args:?}"
        );
    }
}
