//! Maintained resource/dynamic adoption: original official source fixtures plus
//! closed remote provider, checked compile_v3 programs and installed Ruby bytes.
#![cfg(feature = "ruby-sdk")]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::ruby_sdk::{self, PackageConfig, RubyConfig};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, Workspace, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.ruby.test/api.json";
const SCHEMA: &str = "https://physical.ruby.test/official-schema.json";
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn id(uri: &str, segments: &[&str]) -> SchemaId {
    let mut id = SchemaId::new(Uri::parse(uri).unwrap(), Default::default());
    for segment in segments {
        id = id.child(segment)
    }
    id
}
fn schema_id(name: &str) -> SchemaId {
    id(ENTRY, &["components", "schemas", name])
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Ruby resources","version":"1"},"servers":[{"url":"https://service.example.test"}],"paths":{},"components":{"schemas":schemas}})
}
fn load(entry: Value, external: Vec<(&str, Value)>) -> (Arc<Contract>, Arc<Workspace>) {
    let mut documents = vec![(ENTRY, entry)];
    documents.extend(external);
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(uri, value)| {
            ProvidedDocument::new(
                Uri::parse(uri).unwrap(),
                Uri::parse(uri).unwrap(),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap());
    (contract, workspace)
}
fn program(c: Arc<Contract>, roots: &[SchemaId], cfg: Config) -> OwnedProgram {
    let p = OwnedCompiler::new(cfg)
        .compile_v3(c, roots)
        .unwrap()
        .program();
    p.check().unwrap();
    assert_eq!(p.version, OwnedProgram::V3_VERSION);
    p
}
// Keep source root, instance, outcome, limits and finding location visible in
// each independent fixture row, alongside its shared output collection.
#[allow(clippy::too_many_arguments)]
fn add(
    cases: &mut Vec<Value>,
    label: &str,
    c: Arc<Contract>,
    roots: &[SchemaId],
    instance: &str,
    expected: &str,
    cfg: Config,
    source: Option<&str>,
) {
    let p = program(c, roots, cfg);
    let target = p
        .roots
        .iter()
        .find(|r| {
            r.source.document == roots[0].document().as_str()
                && r.source.pointer == roots[0].pointer()
        })
        .unwrap()
        .target;
    cases.push(json!({"id":label,"program":p,"rootTarget":target,"instanceJson":instance,"expected":expected,"source":source}));
}
fn ruby_home() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".local/share/mise/installs/ruby/3.3.12")
        })
}
fn tools() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_GEMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target/sdk-ruby-tools/gems"))
}
fn default_gems() -> PathBuf {
    std::fs::read_dir(ruby_home().join("lib/ruby/gems"))
        .unwrap()
        .map(|d| d.unwrap().path())
        .find(|p| p.is_dir())
        .unwrap()
}
fn ruby() -> Command {
    let mut c = Command::new(ruby_home().join("bin/ruby"));
    c.env("GEM_HOME", tools())
        .env(
            "GEM_PATH",
            format!("{}:{}", tools().display(), default_gems().display()),
        )
        .env(
            "PATH",
            format!(
                "{}:{}",
                ruby_home().join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB");
    c
}
fn checked(c: &mut Command, dir: &Path, label: &str) {
    let o = c.output().unwrap();
    let text = format!(
        "{c:?}\n{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    std::fs::write(dir.join(format!("{label}.log")), &text).unwrap();
    assert!(o.status.success(), "{}\n{text}", dir.display());
    assert!(!text.contains("[error]"), "{text}");
}
fn package(plan: &ruby_sdk::SdkPlan, label: &str) -> PathBuf {
    let base = root().join("target/sdk-ruby-schema-v3-native");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep();
    let p = PackageConfig {
        name: "ruby-schema-v3-gate".into(),
        version: "0.1.0".into(),
        require_name: "ruby_schema_v3".into(),
        namespace: "RubySchemaV3".into(),
    };
    for file in ruby_sdk::emit_sdk(plan, &p).unwrap() {
        let target = dir.join(file.path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, file.content).unwrap();
    }
    checked(ruby().arg("-v"), &dir, "ruby-version");
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["build", "ruby-schema-v3-gate.gemspec"])
            .current_dir(dir.join("ruby")),
        &dir,
        "gem-build",
    );
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["install", "--local", "--no-document"])
            .arg(dir.join("ruby/ruby-schema-v3-gate-0.1.0.gem"))
            .env("GEM_HOME", dir.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    dir.join("installed").display(),
                    default_gems().display()
                ),
            ),
        &dir,
        "gem-install",
    );
    let installed = dir.join("installed/gems/ruby-schema-v3-gate-0.1.0");
    for file in ruby_sdk::emit_sdk(plan, &p).unwrap() {
        let relative = file.path.strip_prefix("ruby/").unwrap();
        if !relative.ends_with(".gemspec") {
            assert_eq!(
                std::fs::read(installed.join(relative)).unwrap(),
                file.content.as_bytes(),
                "installed artifact differs: {relative}"
            );
        }
    }
    std::fs::write(dir.join("production-assets.json"),serde_json::to_vec_pretty(&ruby_sdk::source_assets().iter().map(|(name,bytes)|json!({"path":name,"sha256":format!("{:x}",Sha256::digest(bytes))})).collect::<Vec<_>>()).unwrap()).unwrap();
    eprintln!("Ruby v3 native package: {}", dir.display());
    dir
}

fn candidate() -> RubyConfig {
    RubyConfig {
        schema_resources: true,
        dynamic_schema_references: true,
        ..Default::default()
    }
}
fn sdk_contract() -> (Arc<Contract>, Arc<Workspace>) {
    load(
        serde_json::from_str(include_str!("../src/ruby_sdk/tests/schema_v3.openapi.json")).unwrap(),
        vec![],
    )
}
fn resource_sdk() -> ruby_sdk::SdkPlan {
    let (contract, ws) = sdk_contract();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = ruby_sdk::plan_sdk(contract, &selected, RubyConfig::default())
        .unwrap_or_else(|e| panic!("{e:#?}"));
    assert!(ws.failed_document_uris().is_empty());
    plan
}

#[test]
fn resource_native_descriptors_and_profile_selection_preserve_physical_identity() {
    use ruby_sdk::{ModelShape, SampleValue};
    use suspect_codegen::http_protocol::Capability;
    use suspect_codegen::{
        backend::{self, Backend, TargetConfig},
        compatibility::{self, PlanStatus},
    };
    let plan = resource_sdk();
    let selection = plan
        .contract()
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let explicit = ruby_sdk::plan_sdk(plan.contract().clone(), &selection, candidate()).unwrap();
    let identity = PackageConfig {
        name: "ruby-schema-v3-gate".into(),
        version: "0.1.0".into(),
        require_name: "ruby_schema_v3".into(),
        namespace: "RubySchemaV3".into(),
    };
    let files = ruby_sdk::emit_sdk(&plan, &identity).unwrap();
    let opt_in = ruby_sdk::emit_sdk(&explicit, &identity).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| (&file.path, &file.content))
            .collect::<Vec<_>>(),
        opt_in
            .iter()
            .map(|file| (&file.path, &file.content))
            .collect::<Vec<_>>()
    );
    // Optional release attestation; maintained source tests never require an
    // earlier target directory. Compare default emission with native-witnessed
    // installed bytes after capability promotion, without replaying runtimes.
    if let Some(paths) = std::env::var_os("SUSPECT_RUBY_RESOURCE_COMPARE_INSTALLED") {
        let installed = std::env::split_paths(&paths).collect::<Vec<_>>();
        for directory in &installed {
            for file in &files {
                let relative = file.path.strip_prefix("ruby/").unwrap();
                if !relative.ends_with(".gemspec") {
                    assert_eq!(
                        std::fs::read(directory.join(relative)).unwrap(),
                        file.content.as_bytes(),
                        "default emission differs from installed witness: {relative}"
                    );
                }
            }
        }
        let directory = tempfile::Builder::new()
            .prefix("promotion-")
            .tempdir_in(root().join("target/sdk-ruby-schema-v3-verification"))
            .unwrap()
            .keep();
        std::fs::write(directory.join("artifact-identity.json"), serde_json::to_vec_pretty(&json!({
            "profile":plan.program().profile,"installed":installed,
            "artifacts":files.iter().map(|f| json!({"path":f.path,"sha256":format!("{:x}",Sha256::digest(f.content.as_bytes()))})).collect::<Vec<_>>()
        })).unwrap()).unwrap();
        eprintln!(
            "Default capability promotion byte identity: {}",
            directory.display()
        );
    }
    assert_eq!(plan.program().version, OwnedProgram::V3_VERSION);
    assert_eq!(plan.program().profile, OwnedProgram::V3_PROFILE);
    for cap in [
        Capability::SchemaResources,
        Capability::DynamicSchemaReferences,
        Capability::DocumentRelativeServers,
    ] {
        assert!(plan.protocol().capabilities().supports(cap));
    }
    let target = TargetConfig {
        backend: Backend::RubyHttp,
        package_name: "ruby-schema-v3-gate".into(),
        package_version: "0.1.0".into(),
        import_name: Some("RubySchemaV3".into()),
    };
    let canonical = backend::generate(plan.contract().clone(), &selection, &target).unwrap();
    let wire_program: Value = serde_json::from_str(
        &canonical
            .iter()
            .find(|file| file.path.ends_with("/validation-program.json"))
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(wire_program["version"], OwnedProgram::V3_VERSION);
    let captured = compatibility::snapshot(plan.contract().clone(), &[], &[target]).unwrap();
    let native = &captured.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert_eq!(native.runtime.profile, "ruby-http-protocol-v1");
    for name in ["ruby_sdk/resource_guard.rb", "ruby_sdk/validation_v3.rb"] {
        assert!(
            native
                .runtime
                .fingerprinted_assets
                .iter()
                .any(|asset| asset == name)
        );
    }
    assert!(native.models.iter().any(|model| {
        model.descriptor.as_ref().is_some_and(|d| {
            d["type"]["kind"] == "resource-scoped-json"
                && d["type"]["selection"] == "runtime-entered-resource-context"
        })
    }));
    assert!(native.models.iter().any(|model| {
        model.role == "codec"
            && model
                .descriptor
                .as_ref()
                .is_some_and(|d| d["validationVersion"] == OwnedProgram::V3_VERSION)
    }));
    assert!(plan.protocol().codec_schema_closure().len() > plan.protocol().codec_roots().len());
    let tree = plan.models().source_symbol(&schema_id("Tree")).unwrap();
    let ModelShape::Object { fields, .. } = &tree.shape else {
        panic!("named tree model")
    };
    assert!(fields.iter().any(|f| f.wire_name == "label" && f.required));
    assert!(fields.iter().any(|f| f.wire_name == "note" && !f.required));
    let dynamic = plan
        .models()
        .source_symbol(
            &schema_id("Tree")
                .child("properties")
                .child("children")
                .child("items"),
        )
        .unwrap();
    let ModelShape::Dynamic {
        initial_target,
        initial_resource,
        anchor,
        candidates,
    } = &dynamic.shape
    else {
        panic!("resource-scoped JSON carrier")
    };
    assert_eq!(anchor.as_deref(), Some("node"));
    assert_eq!(
        plan.program().nodes[*initial_target].source.pointer,
        "/components/schemas/Tree"
    );
    let context = plan.program().resource_context.as_ref().unwrap();
    assert_eq!(
        context.resources[*initial_resource].canonical_uri,
        "urn:ruby:tree"
    );
    assert!(
        candidates
            .iter()
            .any(|&i| plan.program().nodes[i].source.pointer == "/components/schemas/Inert")
    );
    assert!(plan.models().source_symbol(&schema_id("Inert")).is_none());
    assert!(
        context
            .node_scopes
            .iter()
            .any(|(_, _, address)| address.contains("a~1b~0%F0%9F%98%80%25"))
    );
    assert!(
        plan.program()
            .nodes
            .iter()
            .all(|node| node.source.document == ENTRY)
    );
    assert!(context.resources.iter().any(|r| r.canonical_uri
        == "https://logical.ruby.test/catalog/api.json"
        && r.source.document == ENTRY));
    assert_eq!(plan.examples().format(), "suspect-sdk-examples-v2");
    assert!(
        plan.native_examples()
            .iter()
            .all(|example| matches!(example.value, SampleValue::Decoded { .. }))
    );
    for operation in ["saveTree", "choose", "identify"] {
        let source = &plan
            .operations()
            .iter()
            .find(|op| op.operation_id == operation)
            .unwrap()
            .source;
        assert!(
            plan.examples()
                .operations()
                .iter()
                .any(|op| &op.source == source && !op.entries.is_empty())
        );
    }
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .filter(|d| d.code == "examples-declared-invalid")
            .count()
            >= 2
    );
    for (schema, dynamic, name) in [
        (false, true, "SchemaResources"),
        (true, false, "DynamicSchemaReferences"),
    ] {
        let (c, _) = sdk_contract();
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let refused = ruby_sdk::plan_sdk(
            c.clone(),
            &selected,
            RubyConfig {
                schema_resources: schema,
                dynamic_schema_references: dynamic,
                ..candidate()
            },
        )
        .unwrap_err();
        assert!(
            refused.iter().any(|e| e.code == "http-capability-required"
                && e.message.contains(name)
                && e.source.document().as_str() == ENTRY
                && Some(e.at.clone()) == c.source_span(&e.source)),
            "{refused:#?}"
        );
    }
    // Capability availability must not change ordinary portable envelopes/bytes.
    for (schema, version) in [
        (
            json!({"type":"integer","minimum":0}),
            OwnedProgram::V1_VERSION,
        ),
        (
            json!({"type":"object","properties":{"a":true},"unevaluatedProperties":false}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let mut doc = api(json!({"Root":schema}));
        doc["paths"] = json!({"/check":{"post":{"operationId":"check","requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}},"responses":{"200":{}}}}});
        let (c, _) = load(doc, vec![]);
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let plain = ruby_sdk::plan_sdk(c.clone(), &selected, candidate()).unwrap();
        let old = OwnedCompiler::new(candidate().schema)
            .compile_v2(c, plain.protocol().codec_schema_closure())
            .unwrap()
            .program();
        assert_eq!(plain.program().version, version);
        assert_eq!(
            serde_json::to_vec(plain.program()).unwrap(),
            serde_json::to_vec(&old).unwrap()
        );
        assert!(plain.program().resource_context.is_none());
    }
    for (schema, keyword) in [
        (
            json!({"$id":"urn:bad","$recursiveRef":"#"}),
            "$recursiveRef",
        ),
        (
            json!({"$id":"urn:bad","$vocabulary":{"https://custom.ruby.test/vocab":true}}),
            "$vocabulary",
        ),
        (
            json!({"$schema":"https://custom.ruby.test/dialect","$id":"urn:bad"}),
            "$id",
        ),
    ] {
        let mut doc = api(json!({"Root":schema}));
        doc["paths"] = json!({"/check":{"post":{"operationId":"check","requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}},"responses":{"200":{}}}}});
        let (c, _) = load(doc, vec![]);
        let selected = c
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let errors = ruby_sdk::plan_sdk(c.clone(), &selected, candidate()).unwrap_err();
        // Shared diagnostics may identify the containing schema while retaining
        // the exact keyword value span. Forward both pieces without rewriting.
        let declared = c.source_span(&schema_id("Root").child(keyword)).unwrap();
        assert!(
            errors
                .iter()
                .any(|e| e.source.document().as_str() == ENTRY && e.at == declared),
            "{errors:#?}"
        );
    }
}

#[test]
#[ignore = "requires Ruby, local gem installation, RBS/Steep/YARD; native resource-aware SDK, executable examples and real HTTP"]
fn installed_resource_sdk_models_types_examples_and_wire() {
    let plan = resource_sdk();
    let dir = package(&plan, "sdk-");
    consumer(
        &dir,
        include_str!("../src/ruby_sdk/tests/schema_v3_sdk.rb"),
        "sdk",
    );
    let installed = dir.join("installed/gems/ruby-schema-v3-gate-0.1.0");
    checked(
        ruby()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed.join("sig"))
            .arg("validate"),
        &dir,
        "rbs",
    );
    checked(
        ruby()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(&installed),
        &dir,
        "yard",
    );
    for name in ["Tree", "Choice", "Outer", "Named"] {
        assert!(
            installed
                .join(format!("doc/RubySchemaV3/Models/{name}.html"))
                .is_file()
        );
    }
    checked(
        ruby()
            .arg(installed.join("examples/contract_examples.rb"))
            .env("GEM_HOME", dir.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    dir.join("installed").display(),
                    default_gems().display()
                ),
            ),
        &dir,
        "examples",
    );
    let types = dir.join("types");
    std::fs::create_dir_all(&types).unwrap();
    std::fs::write(
        types.join("Steepfile"),
        "target :consumer do\n  library 'ruby-schema-v3-gate'\n  check 'consumer.rb'\nend\n",
    )
    .unwrap();
    std::fs::write(
        types.join("consumer.rb"),
        include_str!("../src/ruby_sdk/tests/schema_v3_types.rb"),
    )
    .unwrap();
    let gem_path = format!(
        "{}:{}:{}",
        tools().display(),
        dir.join("installed").display(),
        default_gems().display()
    );
    checked(
        ruby()
            .arg(tools().join("bin/steep"))
            .args(["check", "--jobs=1"])
            .env("GEM_PATH", &gem_path)
            .current_dir(&types),
        &dir,
        "types-positive",
    );
    for (i, snippet) in [
        "RubySchemaV3::Models::Tree.new",
        "RubySchemaV3::Models::Tree.new(label: 1)",
        "RubySchemaV3::Models::Tree.new(label: 'n', note: false)",
        "RubySchemaV3::Models::Tree.new(label: 'n', children: [{'label' => 0.1}])",
        "RubySchemaV3::Models::Choice.new(value: 0.1)",
        "RubySchemaV3::Models::Outer.new(choice: {'value' => 7})",
        "RubySchemaV3::Client.new.save_tree(body: {'label' => 'n'})",
        "RubySchemaV3::Client.new.choose",
    ]
    .iter()
    .enumerate()
    {
        std::fs::write(types.join("consumer.rb"), snippet).unwrap();
        std::fs::write(dir.join(format!("types-negative-{i}.rb")), snippet).unwrap();
        let output = ruby()
            .arg(tools().join("bin/steep"))
            .args(["check", "--jobs=1"])
            .env("GEM_PATH", &gem_path)
            .current_dir(&types)
            .output()
            .unwrap();
        let result = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(dir.join(format!("types-negative-{i}.log")), &result).unwrap();
        assert!(
            !output.status.success()
                && result.contains("Ruby::")
                && !result.contains("UnknownConstant"),
            "negative type consumer did not execute: {snippet}\n{result}"
        );
    }
}
fn consumer(dir: &Path, script: &str, name: &str) {
    std::fs::write(dir.join(format!("{name}.rb")), script).unwrap();
    checked(
        ruby()
            .arg(format!("{name}.rb"))
            .current_dir(dir)
            .env("GEM_HOME", dir.join("installed"))
            .env(
                "GEM_PATH",
                format!(
                    "{}:{}",
                    dir.join("installed").display(),
                    default_gems().display()
                ),
            ),
        dir,
        name,
    );
}

#[test]
#[ignore = "requires native Ruby; compiles all unmodified official source cases via compile_v3, no frozen target input"]
fn source_driven_official_v3_resources_scopes_and_guards() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suspect-schema/tests/fixtures/resource-conformance");
    let read = |name: &str| {
        serde_json::from_slice::<Value>(&std::fs::read(fixtures.join(name)).unwrap()).unwrap()
    };
    let groups = read("dynamicRef.json");
    let mut cases = Vec::new();
    for group in groups.as_array().unwrap() {
        let (c, ws) = load(
            api(json!({"Use":{"$ref":SCHEMA}})),
            vec![
                (SCHEMA, group["schema"].clone()),
                (
                    "http://localhost:1234/draft2020-12/tree.json",
                    read("tree.json"),
                ),
                (
                    "http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",
                    read("extendible-dynamic-ref.json"),
                ),
                (
                    "http://localhost:1234/draft2020-12/detached-dynamicref.json",
                    read("detached-dynamicref.json"),
                ),
            ],
        );
        let root = id(SCHEMA, &[]);
        let p = program(c.clone(), std::slice::from_ref(&root), Config::default());
        assert!(ws.failed_document_uris().is_empty());
        for case in group["tests"].as_array().unwrap() {
            cases.push(json!({"id":format!("{} / {}",group["description"].as_str().unwrap(),case["description"].as_str().unwrap()),"program":p,"rootTarget":p.roots[0].target,"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"Valid"}else{"Invalid"}}));
        }
    }
    assert_eq!(cases.len(), 44);
    for fixture in ["unevaluatedProperties.json", "unevaluatedItems.json"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../suspect-schema/tests/conformance/draft2020-12")
            .join(fixture);
        let groups: Vec<Value> = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for group in groups.iter().filter(|g| {
            matches!(
                g["description"].as_str(),
                Some(
                    "unevaluatedProperties with $dynamicRef" | "unevaluatedItems with $dynamicRef"
                )
            )
        }) {
            let (c, ws) = load(
                api(json!({"Use":{"$ref":SCHEMA}})),
                vec![(SCHEMA, group["schema"].clone())],
            );
            let p = program(c, &[id(SCHEMA, &[])], Config::default());
            assert!(ws.failed_document_uris().is_empty());
            for case in group["tests"].as_array().unwrap() {
                cases.push(json!({"id":format!("{} / {}",group["description"].as_str().unwrap(),case["description"].as_str().unwrap()),"program":p,"rootTarget":p.roots[0].target,"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"Valid"}else{"Invalid"}}));
            }
        }
    }
    assert_eq!(cases.len(), 48);
    let (c, _) = load(
        api(json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#node"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:unentered","$dynamicAnchor":"node","not":{}}
        })),
        vec![],
    );
    for (label, text, expected) in [
        (
            "outermost-entered",
            r#"{"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}"#,
            "Valid",
        ),
        (
            "unentered-does-not-override",
            r#"{"outer":true,"middle":true}"#,
            "Valid",
        ),
        (
            "outermost-child-requirement",
            r#"{"outer":true,"middle":true,"children":[{"middle":true}]}"#,
            "Invalid",
        ),
    ] {
        add(
            &mut cases,
            label,
            c.clone(),
            &[schema_id("Outer"), schema_id("Unentered")],
            text,
            expected,
            Config::default(),
            None,
        );
    }
    let (c, _) = load(
        api(json!({
            "Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},"Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},"Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]},
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},"Outer":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:new"}},"New":{"$id":"urn:new","$defs":{"binding":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"}
        })),
        vec![],
    );
    add(
        &mut cases,
        "failed-trial-scope-restored",
        c.clone(),
        &[schema_id("Trial")],
        "\"text\"",
        "Valid",
        Config::default(),
        None,
    );
    add(
        &mut cases,
        "changed-context-is-not-cycle",
        c,
        &[schema_id("Outer")],
        "7",
        "Valid",
        Config::default(),
        None,
    );
    let outer = "https://physical.ruby.test/outer.json";
    let base = "https://physical.ruby.test/base.json";
    let (c, _) = load(
        api(json!({"Use":{"$ref":format!("{outer}#/$defs/start")}})),
        vec![
            (
                outer,
                json!({"$id":"urn:outer","const":false,"$defs":{"binding":{"$dynamicAnchor":"node","type":"integer"},"start":{"$ref":"urn:base#/$defs/use"}}}),
            ),
            (
                base,
                json!({"$id":"urn:base","$defs":{"target":{"$dynamicAnchor":"node","type":"string"},"use":{"$dynamicRef":"#node"}}}),
            ),
        ],
    );
    let nested = id(outer, &["$defs", "start"]);
    let p = program(c.clone(), std::slice::from_ref(&nested), Config::default());
    assert!(
        !p.nodes
            .iter()
            .any(|n| n.source.document == outer && n.source.pointer.is_empty())
    );
    add(
        &mut cases,
        "nested-entry-does-not-evaluate-resource-root",
        c.clone(),
        std::slice::from_ref(&nested),
        "7",
        "Valid",
        Config::default(),
        None,
    );
    add(
        &mut cases,
        "nested-entry-binds-parent-resource",
        c,
        std::slice::from_ref(&nested),
        "\"x\"",
        "Invalid",
        Config::default(),
        None,
    );
    let (c, _) = load(
        api(json!({"Root":{"$id":"urn:budget","type":"string"}})),
        vec![],
    );
    add(
        &mut cases,
        "resource-enter-exact-cost",
        c.clone(),
        &[schema_id("Root")],
        "\"s\"",
        "Valid",
        Config {
            max_evaluation_steps: 3,
            ..Default::default()
        },
        None,
    );
    add(
        &mut cases,
        "resource-entry-not-free",
        c,
        &[schema_id("Root")],
        "\"s\"",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 2,
            ..Default::default()
        },
        Some("/components/schemas/Root/type"),
    );
    let (c, _) = load(
        api(
            json!({"Root":{"$id":"urn:lookup","$dynamicRef":"#x","$defs":{"X":{"$dynamicAnchor":"x","type":"string"}}}}),
        ),
        vec![],
    );
    add(
        &mut cases,
        "dynamic-lookup-exact-cost",
        c.clone(),
        &[schema_id("Root")],
        "\"s\"",
        "Valid",
        Config {
            max_evaluation_steps: 7,
            ..Default::default()
        },
        None,
    );
    add(
        &mut cases,
        "dynamic-lookup-binding-cost",
        c,
        &[schema_id("Root")],
        "\"s\"",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 6,
            ..Default::default()
        },
        Some("/components/schemas/Root/$defs/X/type"),
    );
    for (name, root_schema) in [
        ("not", json!({"not":{"$dynamicRef":"urn:number#n"}})),
        (
            "anyOf",
            json!({"anyOf":[true,{"$dynamicRef":"urn:number#n"}]}),
        ),
        (
            "if",
            json!({"if":{"$dynamicRef":"urn:number#n"},"then":true,"else":true}),
        ),
    ] {
        let (c, _) = load(
            api(
                json!({"Root":root_schema,"Number":{"$id":"urn:number","$dynamicAnchor":"n","maximum":0}}),
            ),
            vec![],
        );
        add(
            &mut cases,
            &format!("noninvertible-{name}"),
            c,
            &[schema_id("Root")],
            "12345",
            "EvaluationFailure",
            Config {
                max_number_bytes: 3,
                ..Default::default()
            },
            Some("/components/schemas/Number/maximum"),
        );
    }
    let (c, _) = load(
        api(
            json!({"Root":{"$id":"urn:cycle","$dynamicAnchor":"node","anyOf":[true,{"$dynamicRef":"#node"}]}}),
        ),
        vec![],
    );
    add(
        &mut cases,
        "context-cycle-still-noninvertible",
        c,
        &[schema_id("Root")],
        "null",
        "EvaluationFailure",
        Config::default(),
        Some("/components/schemas/Root"),
    );
    additional_controls(&mut cases);
    // The installed runtime vehicle has a normal v1 SDK program. New profiles
    // are witnessed directly before enabling resource admission in plan_sdk.
    let (c, _) = load(
        json!({"openapi":"3.2.0","info":{"title":"Runtime vehicle","version":"1"},"servers":[{"url":"https://service.example.test"}],"paths":{"/ping":{"get":{"operationId":"ping","responses":{"200":{}}}}}}),
        vec![],
    );
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = ruby_sdk::plan_sdk(c, &selected, RubyConfig::default()).unwrap();
    assert_eq!(plan.program().version, OwnedProgram::V1_VERSION);
    let dir = package(&plan, "vectors-");
    let input_hashes = ["dynamicRef.json", "tree.json", "extendible-dynamic-ref.json", "detached-dynamicref.json"].map(|name| json!({"file":format!("crates/suspect-schema/tests/fixtures/resource-conformance/{name}"),"sha256":format!("{:x}",Sha256::digest(std::fs::read(fixtures.join(name)).unwrap()))}));
    std::fs::write(
        dir.join("source-fixtures.json"),
        serde_json::to_vec_pretty(&input_hashes).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("cases.json"),
        serde_json::to_string(&cases).unwrap(),
    )
    .unwrap();
    consumer(
        &dir,
        include_str!("../src/ruby_sdk/tests/schema_v3_vectors.rb"),
        "vectors",
    );
}

fn additional_controls(cases: &mut Vec<Value>) {
    let (c, _) = load(
        api(json!({
            "Root":{"$id":"urn:annotation-root","properties":{"a":true},"$dynamicRef":"urn:annotation-target#target"},
            "Target":{"$id":"urn:annotation-target","$dynamicAnchor":"target","unevaluatedProperties":false}
        })),
        vec![],
    );
    add(
        cases,
        "dynamic-target-starts-fresh",
        c,
        &[schema_id("Root")],
        r#"{"a":1}"#,
        "Invalid",
        Config::default(),
        Some("/components/schemas/Target/unevaluatedProperties"),
    );
    let (c, _) = load(
        api(json!({
            "Root":{"$id":"urn:annotation-root","$dynamicRef":"urn:annotation-target#target","unevaluatedProperties":false},
            "Target":{"$id":"urn:annotation-target","$dynamicAnchor":"target","properties":{"a":true}}
        })),
        vec![],
    );
    add(
        cases,
        "dynamic-success-propagates-annotations",
        c.clone(),
        &[schema_id("Root")],
        r#"{"a":1}"#,
        "Valid",
        Config::default(),
        None,
    );
    add(
        cases,
        "dynamic-success-does-not-mark-unclaimed",
        c,
        &[schema_id("Root")],
        r#"{"a":1,"b":2}"#,
        "Invalid",
        Config::default(),
        Some("/components/schemas/Root/unevaluatedProperties"),
    );
    let (c, _) = load(
        api(json!({
            "Root":{"allOf":[{"$ref":"urn:first"},{"$dynamicRef":"urn:last#x"}]},
            "First":{"$id":"urn:first","type":"integer","$defs":{"Binding":{"$dynamicAnchor":"x","not":{}}}},
            "Last":{"$id":"urn:last","$dynamicAnchor":"x","type":"integer"}
        })),
        vec![],
    );
    add(
        cases,
        "successful-sibling-scope-restored",
        c,
        &[schema_id("Root")],
        "7",
        "Valid",
        Config::default(),
        None,
    );
    for (label, keyword, reference, instance, expected) in [
        (
            "dynamic-pointer-fallback",
            "$dynamicRef",
            "urn:base#/$defs/Leaf",
            "\"s\"",
            "Valid",
        ),
        (
            "dynamic-empty-fragment-fallback",
            "$dynamicRef",
            "urn:base#",
            "\"s\"",
            "Valid",
        ),
        (
            "dynamic-empty-reference-fallback",
            "$dynamicRef",
            "urn:base",
            "\"s\"",
            "Valid",
        ),
        (
            "dynamic-static-anchor-fallback",
            "$dynamicRef",
            "urn:base#plain",
            "\"s\"",
            "Valid",
        ),
        (
            "static-ref-never-rebinds",
            "$ref",
            "urn:base#node",
            "\"s\"",
            "Valid",
        ),
        (
            "plain-name-actually-rebinds",
            "$dynamicRef",
            "urn:base#node",
            "7",
            "Valid",
        ),
        (
            "plain-name-rejects-fallback-kind",
            "$dynamicRef",
            "urn:base#node",
            "\"s\"",
            "Invalid",
        ),
    ] {
        let (c, _) = load(
            api(json!({
                "Root":{"$id":"urn:root","$defs":{"Node":{"$dynamicAnchor":"node","type":"integer"},"Leaf":{"$dynamicAnchor":"leaf","type":"integer"}},(keyword):reference},
                "Base":{"$id":"urn:base","$dynamicAnchor":"node","$anchor":"plain","type":"string","$defs":{"Leaf":{"$dynamicAnchor":"leaf","type":"string"}}}
            })),
            vec![],
        );
        add(
            cases,
            label,
            c,
            &[schema_id("Root")],
            instance,
            expected,
            Config::default(),
            None,
        );
    }
    let (c, _) = load(
        api(json!({
            "Root":{"$id":"urn:override","$dynamicRef":"urn:fallback#x","$defs":{"Binding":{"$dynamicAnchor":"x","type":"integer"}}},
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"x","type":"string"}
        })),
        vec![],
    );
    add(
        cases,
        "no-premature-fallback-resource-entry",
        c,
        &[schema_id("Root")],
        "7",
        "Valid",
        Config {
            max_evaluation_steps: 7,
            ..Default::default()
        },
        None,
    );
    let (c, _) = load(
        api(json!({
            "Root":{"$id":"urn:scan","$dynamicRef":"#z","$defs":{"A":{"$dynamicAnchor":"a"},"B":{"$dynamicAnchor":"b"},"Z":{"$dynamicAnchor":"z","type":"string"}}}
        })),
        vec![],
    );
    let roots = [
        schema_id("Root"),
        schema_id("Root").child("$defs").child("A"),
        schema_id("Root").child("$defs").child("B"),
    ];
    add(
        cases,
        "every-inspected-binding-costs-one",
        c.clone(),
        &roots,
        "\"s\"",
        "Valid",
        Config {
            max_evaluation_steps: 9,
            ..Default::default()
        },
        None,
    );
    add(
        cases,
        "binding-scan-is-not-free",
        c,
        &roots,
        "\"s\"",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 8,
            ..Default::default()
        },
        Some("/components/schemas/Root/$defs/Z/type"),
    );
    let (c, _) = load(
        api(
            json!({"Root":{"$id":"urn:scan-root","$ref":"urn:scan-middle"},"Middle":{"$id":"urn:scan-middle","$dynamicRef":"urn:scan-last#x"},"Last":{"$id":"urn:scan-last","$dynamicAnchor":"x","type":"string"}}),
        ),
        vec![],
    );
    add(
        cases,
        "every-inspected-resource-costs-one",
        c.clone(),
        &[schema_id("Root")],
        "\"s\"",
        "Valid",
        Config {
            max_evaluation_steps: 11,
            ..Default::default()
        },
        None,
    );
    add(
        cases,
        "resource-scan-is-not-free",
        c,
        &[schema_id("Root")],
        "\"s\"",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 10,
            ..Default::default()
        },
        Some("/components/schemas/Last/type"),
    );
    let mut schemas = serde_json::Map::new();
    for i in 0..550 {
        schemas.insert(
            format!("N{i}"),
            json!({"$id":format!("urn:n{i}"),"$ref":format!("urn:n{}",i+1)}),
        );
    }
    schemas.insert("N550".into(), json!({"$id":"urn:n550"}));
    let (c, _) = load(api(Value::Object(schemas)), vec![]);
    add(
        cases,
        "configured-resource-depth-ceiling",
        c,
        &[schema_id("N0")],
        "null",
        "EvaluationFailure",
        Config::default(),
        Some("/components/schemas/N512"),
    );
    // Ordinary pairs remain executable alongside v3, with metadata omitted.
    for (label, schema, instance) in [
        ("retained-v1-envelope", json!({"type":"integer"}), "1.0"),
        (
            "retained-v2-envelope",
            json!({"properties":{"a":true},"unevaluatedProperties":false}),
            r#"{"a":1}"#,
        ),
    ] {
        let (c, _) = load(api(json!({"Root":schema})), vec![]);
        let p = OwnedCompiler::new(Config::default())
            .compile_v2(c, &[schema_id("Root")])
            .unwrap()
            .program();
        assert!(p.resource_context.is_none());
        cases.push(json!({"id":label,"rootTarget":p.roots[0].target,"program":p,"instanceJson":instance,"expected":"Valid"}));
    }
}
