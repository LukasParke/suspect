//! Directional model-codec views through the installed package surface.
//!
//! Covers the non-HTTP `emit` path: a package planned with Neutral, Request
//! and Response views must derive truthful metadata, pack every distinct
//! validation program through the artifact-driven mechanism, and expose all
//! view codecs to an installed consumer. Opt-in native test; missing tools
//! fail loudly when included.

use std::{process::Command, sync::Arc};

use serde_json::json;
use suspect_codegen::typescript::{
    ModelView,
    codecs::{CodecConfig, plan_codecs_with_views},
    package::{PackageConfig, emit},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[test]
#[ignore = "requires pinned native TypeScript, Node 22 and npm tools"]
fn directional_model_package_installs_all_views_and_validation_programs() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.keep();
    let input = directory.join("api.json");
    std::fs::write(
        &input,
        json!({
            "openapi": "3.1.0",
            "info": {"title": "Directional package", "version": "1"},
            "paths": {},
            "components": {"schemas": {"Record": {
                "type": "object",
                "additionalProperties": false,
                "required": ["id", "secret", "name"],
                "properties": {
                    "id": {"type": "string", "readOnly": true},
                    "secret": {"type": "string", "writeOnly": true},
                    "name": {"type": "string"},
                    "note": {"type": ["string", "null"]}
                }
            }}}
        })
        .to_string(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&directory).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    let plan = plan_codecs_with_views(
        contract,
        &roots,
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        CodecConfig::default(),
    )
    .unwrap();
    let files = emit(
        &plan,
        &PackageConfig {
            name: "@fixture/directional-package".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &directory).unwrap();
    let root = directory.join("typescript");
    let npm_global = directory.join("npm-global.config");
    std::fs::write(&npm_global, "").unwrap();
    for args in [
        vec![
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ],
        vec!["run", "build"],
        vec!["pack", "--offline", "--ignore-scripts"],
    ] {
        let output = Command::new("npm")
            .args(&args)
            .current_dir(&root)
            .env("NPM_CONFIG_USERCONFIG", "/dev/null")
            .env("NPM_CONFIG_GLOBALCONFIG", &npm_global)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture {}\n{}{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let consumer = directory.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        "{\"private\":true,\"type\":\"module\"}",
    )
    .unwrap();
    let installed = Command::new("npm")
        .args([
            "install",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ])
        .arg(root.join("fixture-directional-package-0.0.0.tgz"))
        .current_dir(&consumer)
        .env("NPM_CONFIG_USERCONFIG", "/dev/null")
        .env("NPM_CONFIG_GLOBALCONFIG", &npm_global)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "fixture {}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&installed.stdout),
        String::from_utf8_lossy(&installed.stderr)
    );
    std::fs::write(
        consumer.join("consumer.mjs"),
        r#"
import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { codecs, ModelCodecError } from '@fixture/directional-package';
const invalid = (error) => error instanceof ModelCodecError && error.kind === 'invalid';
const installed = new URL('./node_modules/@fixture/directional-package/', import.meta.url);
const metadata = JSON.parse(readFileSync(new URL('package.json', installed)));
assert.equal(metadata.suspect.modelView, 'Neutral+Request+Response');
assert.equal(metadata.suspect.httpClient, false);
// The artifact-driven packing carries every distinct validation program.
for (const program of [
    'validation-program.js',
    'validation-program-request.js',
    'validation-program-response.js',
]) {
    assert.ok(existsSync(new URL(`dist/${program}`, installed)), program);
}
// Neutral keeps complete requiredness in both directions.
const neutral = codecs.RecordCodec.decode('{"id":"i","secret":"s","name":"n"}');
assert.equal(codecs.RecordCodec.encode(neutral), '{"id":"i","secret":"s","name":"n"}');
assert.throws(() => codecs.RecordCodec.decode('{"secret":"s","name":"n"}'), invalid);
// Request view: readOnly presence relaxed; supplied read-only values stay and validate.
const request = codecs.RecordRequestCodec.decode('{"secret":"s","name":"n"}');
assert.equal(Object.hasOwn(request, 'id'), false);
assert.equal(codecs.RecordRequestCodec.encode(request), '{"secret":"s","name":"n"}');
const retained = codecs.RecordRequestCodec.decode('{"id":"srv","secret":"s","name":"n"}');
assert.equal(codecs.RecordRequestCodec.encode(retained), '{"id":"srv","secret":"s","name":"n"}');
assert.throws(() => codecs.RecordRequestCodec.decode('{"id":123,"secret":"s","name":"n"}'), invalid);
// Response view: writeOnly presence relaxed, readOnly required.
const response = codecs.RecordResponseCodec.decode('{"id":"i","name":"n"}');
assert.equal(Object.hasOwn(response, 'secret'), false);
assert.equal(codecs.RecordResponseCodec.encode(response), '{"id":"i","name":"n"}');
assert.throws(() => codecs.RecordResponseCodec.decode('{"name":"n"}'), invalid);
"#,
    )
    .unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let output = Command::new(&node)
        .arg(consumer.join("consumer.mjs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture {}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}
