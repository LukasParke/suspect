//! Native codec documentation is verified against the same public model plan.

use std::{
    path::Path,
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    typescript::codecs::{CodecConfig, CodecPlan, plan_codecs},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn plan(schemas: Value) -> CodecPlan {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(
        &path,
        json!({
            "openapi":"3.1.0", "info":{"title":"Codec documentation","version":"1"},
            "paths":{}, "components":{"schemas":schemas}
        })
        .to_string(),
    )
    .unwrap();
    let contract = contract(&path);
    plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        CodecConfig::default(),
    )
    .unwrap()
}

fn hostile_plan() -> CodecPlan {
    let prose = "    ```ts\nthrow new Error('prose must stay prose');\n```\n<img src=x onerror=alert('source-html')><script>alert('source-script')</script>\n{@link MissingSymbol}\n@example untrusted\n[click](javascript:alert('source-link'))\n*/ export const injected = true; /* &quot; &amp;";
    plan(json!({
        "Hostile":{"type":"object","description":prose,"additionalProperties":false,"required":["quote\"slash\\\n"],"properties":{
            "quote\"slash\\\n":{"type":"integer","description":prose},"é":{"type":["string","null"]}
        }},
        "A B":{"type":"string","description":"{@link Hostile} <iframe src='https://example.invalid/source-frame'></iframe>"},
        "A-B":{"type":"string"},"雪":{"type":"boolean"},
        "Model":{"type":"string","description":"A model whose codec shares the generic codec type name."},
        "ModelCodec":{"type":"boolean","description":"A model name that also exists in the codec module."},
        "ValidationSource":{"type":"string"}
    }))
}

fn manifest(files: &[OutFile]) -> Value {
    serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "typescript/docs-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap()
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn codec_manifest_binds_every_public_model_to_its_codec_and_source() {
    let plan = hostile_plan();
    let files = plan.render();
    let manifest = manifest(&files);
    assert_eq!(manifest["format"], "suspect-typescript-docs-v1");
    assert_eq!(manifest["codecsImplemented"], true);
    assert_eq!(manifest["releaseReady"], false);
    assert_eq!(
        manifest["codecs"].as_array().unwrap().len(),
        plan.models().symbols().len()
    );
    for model in plan.models().symbols() {
        let codec = manifest["codecs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|codec| codec["model"] == model.name())
            .unwrap();
        assert_eq!(codec["name"], format!("{}Codec", model.name()));
        assert_eq!(codec["file"], "model-codecs.ts");
        assert_eq!(
            codec["source"]["document"],
            model.source().document().to_string()
        );
        assert_eq!(codec["source"]["pointer"], model.source().pointer());
    }
    let options: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "typescript/typedoc.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        options["entryPoints"],
        json!(["models.ts", "model-codecs.ts"])
    );
    let module = &files
        .iter()
        .find(|file| file.path == "typescript/model-codecs.ts")
        .unwrap()
        .content;
    assert!(!module.contains("<img src=x"));
    assert!(!module.contains("{@link MissingSymbol}"));
    assert!(!module.contains("@example untrusted"));
}

fn build(root: &Path) -> Output {
    let tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs");
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    Command::new(node)
        .arg(tool)
        .arg(root)
        .output()
        .expect("native documentation requires pinned Node and TypeDoc")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn native_docs(plan: &CodecPlan) {
    let directory = tempfile::tempdir().unwrap();
    let files = plan.render();
    let planned = manifest(&files);
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let root = directory.path().join("typescript");
    let output = build(&root);
    assert!(output.status.success(), "{}", output_text(&output));
    let coverage: Value =
        serde_json::from_slice(&std::fs::read(root.join("docs/coverage.json")).unwrap()).unwrap();
    assert_eq!(coverage["complete"], true);
    assert_eq!(coverage["codecsImplemented"], true);
    assert_eq!(coverage["releaseReady"], false);
    assert_eq!(
        coverage["symbols"].as_array().unwrap().len(),
        planned["symbols"].as_array().unwrap().len()
    );
    assert_eq!(
        coverage["codecs"].as_array().unwrap().len(),
        planned["codecs"].as_array().unwrap().len()
    );
    for codec in coverage["codecs"].as_array().unwrap() {
        assert_eq!(codec["documented"], true);
        assert!(!codec["url"].as_str().unwrap().is_empty());
        assert!(!codec["modelUrl"].as_str().unwrap().is_empty());
        assert!(!codec["apiUrl"].as_str().unwrap().is_empty());
        for method in ["decode", "encode"] {
            assert_eq!(codec["methods"][method]["documented"], true);
            assert!(!codec["methods"][method]["url"].as_str().unwrap().is_empty());
        }
    }
    let output = Command::new("tsc")
        .current_dir(&root)
        .args(["--project", "tsconfig.docs.json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", output_text(&output));
}

#[test]
#[ignore = "requires pinned native Node/TypeDoc and TypeScript"]
fn native_codec_docs_preserve_hostile_prose_collisions_and_actual_methods() {
    native_docs(&hostile_plan());
}

#[test]
#[ignore = "requires pinned native Node/TypeDoc and tracked OpenRouter YAML"]
fn native_codec_docs_cover_tracked_openrouter_caller_and_image_closures() {
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
        .filter(|root| {
            [
                "/components/schemas/ORAnthropicNullableCaller",
                "/components/schemas/AnthropicImageBlockParam",
            ]
            .contains(&root.pointer())
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    native_docs(&plan_codecs(contract, &roots, CodecConfig::default()).unwrap());
}

#[test]
#[ignore = "requires pinned native Node/TypeDoc"]
fn native_gate_rejects_missing_codec_manifest_and_undocumented_methods() {
    let plan = plan(json!({"Flag":{"type":"boolean"},"Other":{"type":"string"}}));
    for mutation in ["manifest", "export", "method", "binding"] {
        let directory = tempfile::tempdir().unwrap();
        let mut files = plan.render();
        match mutation {
            "manifest" => {
                let entry = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/docs-manifest.json")
                    .unwrap();
                let mut manifest: Value = serde_json::from_str(&entry.content).unwrap();
                manifest.as_object_mut().unwrap().remove("codecs");
                entry.content = manifest.to_string();
            }
            "export" => {
                let entry = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/model-codecs.ts")
                    .unwrap();
                entry.content = entry
                    .content
                    .lines()
                    .filter(|line| !line.starts_with("export const FlagCodec:"))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            "binding" => {
                let entry = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/model-codecs.ts")
                    .unwrap();
                let declaration = entry.content.find("export const FlagCodec:").unwrap();
                let offset = declaration + entry.content[declaration..].find(".Flag>").unwrap();
                entry
                    .content
                    .replace_range(offset..offset + ".Flag>".len(), ".Other>");
            }
            "method" => {
                let entry = files
                    .iter_mut()
                    .find(|file| file.path == "typescript/codecs.ts")
                    .unwrap();
                let signature = entry.content.find("    decode(text: string").unwrap();
                let end = entry.content[..signature].rfind("*/").unwrap() + 2;
                let start = entry.content[..end].rfind("/**").unwrap();
                entry.content.replace_range(start..end, "");
            }
            _ => unreachable!(),
        }
        suspect_codegen::write_files(&files, directory.path()).unwrap();
        let output = build(&directory.path().join("typescript"));
        assert!(
            !output.status.success(),
            "missing {mutation} coverage must block the gate"
        );
        let message = output_text(&output);
        let expected = match mutation {
            "manifest" => "Implemented codecs require a plan-derived codec manifest",
            "export" => "Missing/ambiguous TypeDoc codec FlagCodec",
            "method" => "Missing method prose for FlagCodec.decode",
            "binding" => "Codec FlagCodec is not bound to its planned model type",
            _ => unreachable!(),
        };
        assert!(message.contains(expected), "{mutation}: {message}");
    }
}
