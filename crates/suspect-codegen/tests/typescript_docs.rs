//! Native documentation is checked from the canonical plan's public symbols.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::typescript::{ModelPlan, ModelView, plan_models};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Contract {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap()
}

fn hostile_plan() -> ModelPlan {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let description = "    ```ts\nthrow new Error('prose must stay prose');\n```\n<img src=x onerror=alert('source-html')><script>alert('source-script')</script>\n{@link MissingSymbol}\n@example untrusted\n[click](javascript:alert('source-link'))\n*/ export const injected = true; /* &quot; &amp; ";
    std::fs::write(&path,serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":"Docs","version":"1"},"paths":{},"components":{"schemas":{
        "Hostile":{"type":"object","description":description,"additionalProperties":false,"required":["quote\"slash\\\n"],"properties":{
            "quote\"slash\\\n":{"type":"string","description":description},
            "é":{"type":["string","null"]}
        }},
        "A B":{"type":"string","description":"{@link Hostile} <iframe src='https://example.invalid/source-frame'></iframe>"},
        "A-B":{"type":"string"},"雪":{"type":"boolean"}
    }}})).unwrap()).unwrap();
    let contract = contract(&path);
    plan_models(
        &contract,
        contract.schema_roots(),
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
    )
}

fn manifest(plan: &ModelPlan) -> Value {
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    assert!(!plan.release_ready());
    let files = plan.render().unwrap();
    let manifest = files
        .iter()
        .find(|f| f.path == "typescript/docs-manifest.json")
        .expect("native documentation must have a plan-derived coverage manifest");
    serde_json::from_str(&manifest.content).unwrap()
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn documentation_artifacts_share_symbols_sources_and_release_obligations() {
    let plan = hostile_plan();
    let manifest = manifest(&plan);
    assert_eq!(manifest["format"], "suspect-typescript-docs-v1");
    assert_eq!(manifest["releaseReady"], false);
    assert_eq!(manifest["codecsImplemented"], false);
    assert_eq!(
        manifest["symbols"].as_array().unwrap().len(),
        plan.symbols().len()
    );
    for symbol in plan.symbols() {
        let entry = manifest["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == symbol.name())
            .unwrap();
        assert_eq!(entry["source"]["pointer"], symbol.source().pointer());
        assert_eq!(
            entry["source"]["document"],
            symbol.source().document().to_string()
        );
        assert_eq!(entry["file"], "models.ts");
    }
    let files = plan.render().unwrap();
    let code = &files
        .iter()
        .find(|f| f.path == "typescript/models.ts")
        .unwrap()
        .content;
    assert!(code.contains("@remarks"));
    assert!(!code.contains("<img src=x"));
    assert!(!code.contains("{@link MissingSymbol}"));
    assert!(!code.contains("@example untrusted"));
    for file in [
        "typescript/typedoc.json",
        "typescript/tsconfig.docs.json",
        "typescript/docs-readme.md",
    ] {
        assert!(files.iter().any(|f| f.path == file), "missing {file}");
    }
}

fn native_docs(plan: &ModelPlan) {
    let planned = manifest(plan);
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render().unwrap(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs");
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let output = Command::new(node)
        .arg(tool)
        .arg(&root)
        .output()
        .expect("native docs require the pinned Node and TypeDoc toolchain");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let coverage: Value =
        serde_json::from_slice(&std::fs::read(root.join("docs/coverage.json")).unwrap()).unwrap();
    assert_eq!(coverage["complete"], true);
    assert_eq!(
        coverage["symbols"].as_array().unwrap().len(),
        planned["symbols"].as_array().unwrap().len()
    );
    assert_eq!(coverage["releaseReady"], false);
    let output = Command::new("tsc")
        .current_dir(&root)
        .args(["--project", "tsconfig.docs.json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler"]
fn native_declaration_comments_keep_hostile_prose_inert_and_types_faithful() {
    let plan = hostile_plan();
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render().unwrap(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(
        root.join("consumer.ts"),
        r#"import type { Hostile } from './models.js';
// @ts-expect-error: source prose cannot create an export
import { injected } from './models.js';
const valid: Hostile = { 'quote"slash\\\n': 'value', 'é': null };
// @ts-expect-error: the required escaped wire key remains required
const invalid: Hostile = {};
"#,
    )
    .unwrap();
    let output = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--noEmit",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "models.ts",
            "consumer.ts",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires the pinned native Node/TypeDoc documentation toolchain"]
fn native_typedoc_preserves_hostile_prose_and_escaped_symbol_names_as_text() {
    native_docs(&hostile_plan());
}

#[test]
#[ignore = "requires pinned Node/TypeDoc and tracked OpenRouter public YAML"]
fn native_typedoc_covers_tracked_openrouter_caller_and_image_closures() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&path);
    let roots = contract
        .schema_roots()
        .iter()
        .filter(|id| {
            [
                "/components/schemas/ORAnthropicNullableCaller",
                "/components/schemas/AnthropicImageBlockParam",
            ]
            .contains(&id.pointer())
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    native_docs(&plan_models(
        &contract,
        &roots,
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
    ));
}
