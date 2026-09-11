//! Real Ruby gem consumers. Emitted packages are built and installed unchanged;
//! handwritten wire fixtures and consumer types are independent of the emitter.
#![cfg(feature = "ruby-sdk")]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::ruby_sdk::{
    ModelShape, PackageConfig, RubyConfig, SdkPlan, emit_sdk, plan_sdk,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}
fn canonical() -> SdkPlan {
    let contract = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    plan_sdk(
        contract.clone(),
        &selected(&contract),
        RubyConfig::default(),
    )
    .unwrap()
}
fn contract(schemas: Value) -> Arc<Contract> {
    let base = root().join("target/sdk-ruby-fixtures");
    std::fs::create_dir_all(&base).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("spec-")
        .tempdir_in(&base)
        .unwrap()
        .keep();
    let path = directory.join("api.json");
    let paths: serde_json::Map<_, _> = schemas
        .as_object()
        .unwrap()
        .keys()
        .enumerate()
        .map(|(path_index, key)| {
            let pointer = key.replace('~', "~0").replace('/', "~1");
            let encoded = pointer
                .bytes()
                .map(|byte| {
                    if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                        char::from(byte).to_string()
                    } else {
                        format!("%{byte:02X}")
                    }
                })
                .collect::<String>();
            let reference = format!("#/components/schemas/{encoded}");
            (
                format!("/case/{path_index}"),
                json!({
                    "post": {
                        "operationId": key,
                        "requestBody": {"required": false, "content": {
                            "application/json": {"schema": {"$ref": reference}}
                        }},
                        "responses": {"200": {"description": "ok", "content": {
                            "application/json": {"schema": {"$ref": reference}}
                        }}}
                    }
                }),
            )
        })
        .collect();
    std::fs::write(&path, json!({"openapi":"3.1.0","info":{"title":"Ruby contract","version":"1"},"servers":[{"url":"https://api.example.test/v1"}],"security":[{"token":[]}],"paths":paths,"components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"}},"schemas":schemas}}).to_string()).unwrap();
    load(&path)
}
fn package() -> PackageConfig {
    PackageConfig {
        name: "ruby-native-gate".into(),
        require_name: "ruby_native_gate".into(),
        namespace: "RubyNativeGate".into(),
        version: "0.1.0".into(),
    }
}

#[test]
fn complete_package_retains_native_descriptors_and_source_bindings() {
    let plan = canonical();
    assert_eq!(plan.operations().len(), 4);
    assert!(plan.program().check().is_ok());
    let widget = plan
        .models()
        .symbols()
        .find(|s| s.name == "Widget")
        .unwrap();
    let ModelShape::Object { fields, .. } = &widget.shape else {
        panic!("native object")
    };
    let meta = fields.iter().find(|f| f.wire_name == "meta").unwrap();
    assert!(!meta.required);
    assert!(matches!(
        plan.models().symbol(meta.schema_index).unwrap().shape,
        ModelShape::Scalar(_)
    ));
    assert!(plan.schema_source(widget.schema_index).is_some());
    let files = emit_sdk(&plan, &package()).unwrap();
    assert_eq!(files, emit_sdk(&plan, &package()).unwrap());
    for suffix in [
        ".gemspec",
        "/http.rb",
        "/client.rb",
        "/models.rb",
        "/validation-program.json",
        ".rbs",
        "/source-map.json",
        "/examples/contract_examples.rb",
        "/.yardopts",
    ] {
        assert!(
            files.iter().any(|f| f.path.ends_with(suffix)),
            "missing artifact {suffix}"
        );
    }
    assert!(files.iter().all(|f| f.path.starts_with("ruby/")));
    assert_eq!(
        files.len(),
        files
            .iter()
            .map(|f| &f.path)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
}

#[test]
fn invalid_package_policy_and_unproved_shapes_fail_before_artifacts() {
    let plan = canonical();
    for cfg in [
        PackageConfig {
            name: "../escape".into(),
            ..package()
        },
        PackageConfig {
            require_name: "json".into(),
            ..package()
        },
        PackageConfig {
            namespace: "Object".into(),
            ..package()
        },
        PackageConfig {
            version: "1.0.0.beta".into(),
            ..package()
        },
    ] {
        assert!(
            emit_sdk(&plan, &cfg)
                .unwrap_err()
                .iter()
                .all(|d| d.code == "ruby-package-identity")
        );
    }
    let c = contract(
        json!({"Bad":{"allOf":[{"type":"object","properties":{"a":{"type":"string"}}},{"type":"object","properties":{"b":{"type":"string"}}}]}}),
    );
    let errors = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap_err();
    assert!(errors.iter().any(|d| d.code == "ruby-model-representation"
        && !d.source.pointer().is_empty()
        && d.at.end > d.at.start));
    let c = contract(
        json!({"Bad":{"type":"object","properties":{"secret":{"type":"string","writeOnly":true}}}}),
    );
    assert!(
        plan_sdk(c.clone(), &selected(&c), RubyConfig::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code == "ruby-directional-codec-unsupported")
    );
    let c = contract(json!({"Bad":{"$ref":"#/components/schemas/Bad"}}));
    assert!(
        plan_sdk(c.clone(), &selected(&c), RubyConfig::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code == "ruby-model-nonproductive-cycle")
    );
    let c = contract(json!({"Good":{"type":"string"}}));
    for cfg in [
        RubyConfig {
            max_json_depth: 0,
            ..RubyConfig::default()
        },
        RubyConfig {
            max_capture_bytes: usize::MAX,
            ..RubyConfig::default()
        },
    ] {
        assert!(
            plan_sdk(c.clone(), &selected(&c), cfg)
                .unwrap_err()
                .iter()
                .any(|d| d.code == "ruby-resource-policy")
        );
    }
    let huge: Value = serde_json::from_str(&"1".repeat(4097)).unwrap();
    let c = contract(json!({"Large":{"const":huge}}));
    assert!(
        plan_sdk(c.clone(), &selected(&c), RubyConfig::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code == "ruby-numeric-literal-limit")
    );
}

#[test]
fn every_shared_schema_vector_has_an_explicit_native_carrier() {
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    assert_eq!(cases["cases"].as_array().unwrap().len(), 17);
    let schemas = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("Case{i}"), c["schema"].clone()))
        .collect();
    let c = contract(Value::Object(schemas));
    let plan = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap();
    assert_eq!(plan.operations().len(), 17);
    assert!(
        plan.models()
            .symbols()
            .any(|s| matches!(s.shape, ModelShape::RefinedJson))
    );
    assert!(
        plan.models()
            .symbols()
            .any(|s| matches!(s.shape, ModelShape::Literal(_)))
    );
    emit_sdk(&plan, &package()).unwrap();
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
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
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .expect("Ruby's default gem directory")
}
fn installed_gem_path(root: &Path) -> String {
    format!(
        "{}:{}",
        root.join("installed").display(),
        default_gems().display()
    )
}
fn ruby_command() -> Command {
    let mut command = Command::new(ruby_home().join("bin/ruby"));
    command.env(
        "PATH",
        format!(
            "{}:{}",
            ruby_home().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    command
        .env("GEM_HOME", tools())
        .env(
            "GEM_PATH",
            format!("{}:{}", tools().display(), default_gems().display()),
        )
        .env_remove("RUBYOPT")
        .env_remove("RUBYLIB");
    command
}
fn checked(command: &mut Command, artifact: &Path, name: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    let text = format!(
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(artifact.join(format!("{name}.log")), &text).unwrap();
    assert!(
        output.status.success(),
        "Ruby gate retained at {}\n{text}",
        artifact.display()
    );
    if name == "yard-doc" {
        assert!(
            !text.contains("[error]"),
            "YARD reported an error despite its exit status: {text}"
        );
    }
    text
}
fn native_package(plan: &SdkPlan, label: &str) -> PathBuf {
    let base = root().join("target/sdk-ruby-native");
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix(&format!("{label}-"))
        .tempdir_in(base)
        .unwrap()
        .keep();
    for file in emit_sdk(plan, &package()).unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    checked(ruby_command().arg("-v"), &root, "ruby-version");
    checked(
        ruby_command()
            .arg(ruby_home().join("bin/gem"))
            .args(["build", "ruby-native-gate.gemspec"])
            .current_dir(root.join("ruby")),
        &root,
        "gem-build",
    );
    checked(
        ruby_command()
            .arg(ruby_home().join("bin/gem"))
            .args(["install", "--local", "--no-document"])
            .arg(root.join("ruby/ruby-native-gate-0.1.0.gem"))
            .env("GEM_HOME", root.join("installed"))
            .env("GEM_PATH", installed_gem_path(&root)),
        &root,
        "gem-install",
    );
    root
}
fn installed(root: &Path) -> PathBuf {
    root.join("installed/gems/ruby-native-gate-0.1.0")
}
fn consumer(root: &Path, filename: &str, code: &str) {
    let path = root.join("consumer");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join(filename), code).unwrap();
    checked(
        ruby_command()
            .arg(filename)
            .current_dir(&path)
            .env("GEM_HOME", root.join("installed"))
            .env("GEM_PATH", installed_gem_path(root)),
        root,
        filename,
    );
}
fn signatures_docs_examples(root: &Path) {
    let pkg = installed(root);
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(pkg.join("sig"))
            .arg("validate"),
        root,
        "rbs-validate",
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(&pkg),
        root,
        "yard-doc",
    );
    assert!(pkg.join("doc/RubyNativeGate/Client.html").is_file());
    assert!(pkg.join("doc/RubyNativeGate/Models/Widget.html").is_file());
    let client = std::fs::read_to_string(pkg.join("doc/RubyNativeGate/Client.html")).unwrap();
    assert!(client.contains("create_widget") && client.contains("canonical.openapi.yaml"));
    checked(
        ruby_command()
            .arg(pkg.join("examples/contract_examples.rb"))
            .current_dir(root)
            .env("GEM_HOME", root.join("installed"))
            .env("GEM_PATH", installed_gem_path(root)),
        root,
        "executable-examples",
    );
}
fn typecheck(root: &Path) {
    let dir = root.join("types");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Steepfile"),
        "target :consumer do\n  library 'ruby-native-gate'\n  check 'consumer.rb'\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("consumer.rb"),
        include_str!("../src/ruby_sdk/tests/types.rb"),
    )
    .unwrap();
    checked(
        ruby_command()
            .arg(tools().join("bin/steep"))
            .args(["check", "--jobs=1"])
            .env(
                "GEM_PATH",
                format!("{}:{}", tools().display(), installed_gem_path(root)),
            )
            .current_dir(&dir),
        root,
        "types-positive",
    );
    for (i, source) in [
        "RubyNativeGate::Models::WidgetInput.new\n",
        "RubyNativeGate::Models::WidgetInput.new(name: 42)\n",
        "RubyNativeGate::Models::WidgetPatch.new(amount: nil)\n",
        "RubyNativeGate::Models::StandardPayload.new(text: 'x', kind: 'secure')\n",
        "RubyNativeGate::Client.new(auth: {'apiKey' => 'token'}).get_widget\n",
        "RubyNativeGate::Client.new(auth: {'apiKey' => 'token'}).create_widget(body: 'wrong')\n",
        "RubyNativeGate::Models::Widget.new(id: 'x', amount: 1, payload: RubyNativeGate::Models::WidgetNode.new(label: 'wrong'))\n",
    ].iter().enumerate() {
        std::fs::write(dir.join("consumer.rb"), source).unwrap();
        let output = ruby_command().arg(tools().join("bin/steep")).args(["check", "--jobs=1"]).env("GEM_PATH", format!("{}:{}", tools().display(), installed_gem_path(root))).current_dir(&dir).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        std::fs::write(root.join(format!("types-negative-{i}.log")), &text).unwrap();
        assert!(!output.status.success() && text.contains("Ruby::") && !text.contains("UnknownConstant"), "negative consumer unexpectedly accepted/crashed: {source}\n{text}");
    }
}

#[test]
#[ignore = "requires Ruby 3.3.12+/4.0, isolated YARD/RBS/Steep tools; builds and installs a real gem"]
fn native_m2_gem_types_docs_examples_and_independent_wire() {
    let root = native_package(&canonical(), "m2");
    signatures_docs_examples(&root);
    typecheck(&root);
    consumer(&root, "m2.rb", include_str!("../src/ruby_sdk/tests/m2.rb"));
    consumer(
        &root,
        "transport.rb",
        include_str!("../src/ruby_sdk/tests/transport.rb"),
    );
}

#[test]
#[ignore = "requires native Ruby; executes all 17 shared schemas plus independent adversarial vectors"]
fn native_shared_contract_exact_json_and_adversarial_models() {
    let cases: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let mut schemas: serde_json::Map<_, _> = cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("Case{i}"), c["schema"].clone()))
        .collect();
    schemas.extend(
        serde_json::from_str::<Value>(include_str!("../src/ruby_sdk/tests/adversarial.json"))
            .unwrap()
            .as_object()
            .unwrap()
            .clone(),
    );
    let c = contract(Value::Object(schemas));
    let plan = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap();
    let root = native_package(&plan, "contract");
    std::fs::create_dir_all(root.join("consumer")).unwrap();
    std::fs::write(
        root.join("consumer/runtime-contract-v1.json"),
        cases.to_string(),
    )
    .unwrap();
    consumer(
        &root,
        "contract.rb",
        include_str!("../src/ruby_sdk/tests/contract.rb"),
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed(&root).join("sig"))
            .arg("validate"),
        &root,
        "rbs-validate",
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(installed(&root)),
        &root,
        "yard-doc",
    );
    let docs =
        std::fs::read_to_string(installed(&root).join("doc/RubyNativeGate/Models/Reserved.html"))
            .unwrap();
    assert!(!docs.contains("<script>alert") && docs.contains("&lt;script&gt;"));
}

#[test]
#[ignore = "requires native Ruby; independent finite-budget contracts run through installed codecs"]
fn native_shared_branch_copy_equality_and_number_budgets() {
    let cases = [
        (
            "evaluation",
            json!({"anyOf":(0..12).map(|_| json!({"type":"number"})).collect::<Vec<_>>()}),
            RubyConfig {
                schema: suspect_schema::Config {
                    max_evaluation_steps: 16,
                    max_depth: 128,
                    ..Default::default()
                },
                ..Default::default()
            },
            "1",
        ),
        (
            "conversion",
            json!({"anyOf":(0..12).map(|_| json!({"type":"array","items":{"type":"integer","minimum":1}})).collect::<Vec<_>>()}),
            RubyConfig {
                max_conversion_steps: 30,
                ..Default::default()
            },
            "[0]",
        ),
        (
            "equality",
            json!({"anyOf":[{"const":[1,2,3]},{"const":[1,2,3]}]}),
            RubyConfig {
                schema: suspect_schema::Config {
                    max_equality_steps: 5,
                    max_depth: 128,
                    ..Default::default()
                },
                ..Default::default()
            },
            "[1,2,3]",
        ),
        (
            "literal-number",
            json!({"const":1000000000000u64}),
            RubyConfig {
                schema: suspect_schema::Config {
                    max_number_bytes: 4,
                    max_depth: 128,
                    ..Default::default()
                },
                ..Default::default()
            },
            "1000000000000",
        ),
        (
            "copy",
            json!(true),
            RubyConfig {
                max_conversion_steps: 40,
                ..Default::default()
            },
            "[\"abcdefghij\",\"abcdefghij\",\"abcdefghij\",\"abcdefghij\"]",
        ),
    ];
    for (label, schema, config, value) in cases {
        let c = contract(json!({"Budget":schema}));
        let plan = plan_sdk(c.clone(), &selected(&c), config).unwrap();
        let root = native_package(&plan, label);
        consumer(
            &root,
            "budget.rb",
            &format!(
                "require 'ruby_native_gate'\ninclude RubyNativeGate\nbegin\n  Codecs::Budget.encode(Json.parse({}))\nrescue EvaluationFailure => error\n  raise 'missing source' unless error.source.include?('/components/schemas/Budget')\n  puts 'shared {label} budget passed'\nelse\n  raise 'budget reset or swallowed'\nend\n",
                serde_json::to_string(value).unwrap()
            ),
        );
    }
}

#[test]
#[ignore = "requires tracked OpenRouter source and native Ruby; five real operations with hand-authored HTTP responses"]
fn native_openrouter_five_actual_operations() {
    let web = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT to the tracked repository");
    let c = load(&web.join("projects/docs/openapi/openapi.yaml"));
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = c
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    let plan = plan_sdk(c, &selected, RubyConfig::default()).unwrap();
    let root = native_package(&plan, "openrouter");
    std::fs::create_dir_all(root.join("consumer")).unwrap();
    std::fs::write(
        root.join("consumer/responses.json"),
        include_str!("fixtures/openrouter-five-responses.json"),
    )
    .unwrap();
    let create = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "createKeys")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .schema_index()
        .unwrap();
    let update = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "updateKeys")
        .unwrap()
        .body
        .as_ref()
        .unwrap()
        .media[0]
        .schema_index()
        .unwrap();
    consumer(
        &root,
        "openrouter.rb",
        &include_str!("../src/ruby_sdk/tests/openrouter.rb")
            .replace("__CREATE__", &plan.models().carrier(create).name)
            .replace("__UPDATE__", &plan.models().carrier(update).name),
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed(&root).join("sig"))
            .arg("validate"),
        &root,
        "rbs-validate",
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(installed(&root)),
        &root,
        "yard-doc",
    );
    assert!(
        installed(&root)
            .join("doc/RubyNativeGate/Client.html")
            .is_file()
    );
    checked(
        ruby_command()
            .arg(installed(&root).join("examples/contract_examples.rb"))
            .current_dir(&root)
            .env("GEM_HOME", root.join("installed"))
            .env("GEM_PATH", installed_gem_path(&root)),
        &root,
        "executable-examples",
    );
}

fn expanded_protocol() -> (SdkPlan, Value) {
    let mut doc: Value =
        serde_json::from_str(include_str!("../src/ruby_sdk/tests/protocol.openapi.json")).unwrap();
    // These deliberately undefined declarations are asserted separately below.
    doc["paths"].as_object_mut().unwrap().remove("/conflict");
    doc["paths"]["/headers"]["get"]["responses"]["200"]["headers"]
        .as_object_mut()
        .unwrap()
        .remove("Set-Cookie");
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    for (i, vector) in vectors["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let path = if vector["parameter"]["in"] == "path" {
            format!("/vectors/{i}/{{color}}")
        } else {
            format!("/vectors/{i}")
        };
        doc["paths"][path] = json!({"get":{"operationId":format!("vector{i}"),"parameters":[vector["parameter"].clone()],"responses":{"200":{}}}});
    }
    doc["paths"]["/read-form"] = json!({"get":{"operationId":"readForm","responses":{"200":{"content":doc["paths"]["/form"]["post"]["requestBody"]["content"].clone()}}}});
    doc["paths"]["/read-parts"] = json!({"get":{"operationId":"readParts","responses":{"200":{"content":doc["paths"]["/upload"]["post"]["requestBody"]["content"].clone()}}}});
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
    ] {
        doc["paths"]["/methods"][method] =
            json!({"operationId":format!("method{method}"),"responses":{"200":{}}});
    }
    for method in ["COPY", "GeT", "head", "x-PING"] {
        doc["paths"]["/methods"]["additionalOperations"][method] =
            json!({"operationId":format!("custom{method}"),"responses":{"200":{}}});
    }
    let base = root().join("target/sdk-ruby-protocol-fixtures");
    std::fs::create_dir_all(&base).unwrap();
    let path = tempfile::Builder::new()
        .prefix("protocol-")
        .tempdir_in(&base)
        .unwrap()
        .keep()
        .join("api.json");
    std::fs::write(&path, doc.to_string()).unwrap();
    let c = load(&path);
    let plan = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap();
    (plan, vectors)
}

#[test]
fn protocol_planning_uses_real_codec_roots_and_native_wire_records() {
    let (plan, _) = expanded_protocol();
    assert!(plan.protocol().is_admitted());
    assert_eq!(plan.examples().format(), "suspect-sdk-examples-v2");
    let upload = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "upload")
        .unwrap();
    assert!(matches!(
        upload.body.as_ref().unwrap().media[0].value_type,
        suspect_codegen::ruby_sdk::NativeType::Record(_)
    ));
    assert!(!plan.program().roots.iter().any(|r| {
        r.source
            .pointer
            .ends_with("/multipart~1form-data/schema/properties/file")
    }));
    assert!(plan.records().iter().any(|r| r.name == "UploadRequest"));
    assert!(plan.records().iter().any(|r| r.name.contains("Headers")));
    assert!(plan.operations().iter().any(|o| o.method == "GeT"));
    emit_sdk(&plan, &package()).unwrap();
}

#[test]
fn protocol_declines_conflicting_credentials_and_undefined_cookie_repetition() {
    let doc: Value =
        serde_json::from_str(include_str!("../src/ruby_sdk/tests/protocol.openapi.json")).unwrap();
    let base = root().join("target/sdk-ruby-protocol-fixtures");
    std::fs::create_dir_all(&base).unwrap();
    let path = tempfile::Builder::new()
        .prefix("decline-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .join("api.json");
    std::fs::write(&path, doc.to_string()).unwrap();
    let c = load(&path);
    let errors = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "http-security-attachment-conflict"
                && e.source.pointer().contains("/security/")
                && e.at.end > e.at.start)
    );
    assert!(errors.iter().any(|e| e.code == "http-set-cookie-repetition"
        && e.source.pointer().ends_with("/headers/Set-Cookie")));
}

#[test]
#[ignore = "requires native Ruby, installed gem, YARD/RBS/Steep and real loopback HTTP"]
fn native_expanded_protocol_wire_parts_streams_and_types() {
    let (plan, fixture) = expanded_protocol();
    let root = native_package(&plan, "protocol");
    std::fs::create_dir_all(root.join("consumer")).unwrap();
    let bindings=fixture["parameterCases"].as_array().unwrap().iter().enumerate().map(|(i,v)|{
        let op=plan.operations().iter().find(|o|o.operation_id==format!("vector{i}")).unwrap();let p=&op.parameters[0];
        json!({"method":op.method_name,"keyword":p.keyword,"codec":plan.models().symbol(p.schema_index().unwrap()).unwrap().name,"vector":v})
    }).collect::<Vec<_>>();
    std::fs::write(
        root.join("consumer/vectors.json"),
        serde_json::to_string(&bindings).unwrap(),
    )
    .unwrap();
    let methods = plan
        .operations()
        .iter()
        .filter(|o| o.path == "/methods")
        .map(|o| json!({"method":o.method,"call":o.method_name}))
        .collect::<Vec<_>>();
    std::fs::write(
        root.join("consumer/methods.json"),
        serde_json::to_string(&methods).unwrap(),
    )
    .unwrap();
    consumer(
        &root,
        "protocol.rb",
        include_str!("../src/ruby_sdk/tests/protocol.rb"),
    );
    consumer(
        &root,
        "streaming.rb",
        include_str!("../src/ruby_sdk/tests/streaming.rb"),
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed(&root).join("sig"))
            .arg("validate"),
        &root,
        "rbs-validate",
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(installed(&root)),
        &root,
        "yard-doc",
    );
    assert!(
        installed(&root)
            .join("doc/RubyNativeGate/Models/UploadRequest.html")
            .is_file()
    );
    checked(
        ruby_command()
            .arg(installed(&root).join("examples/contract_examples.rb"))
            .env("GEM_HOME", root.join("installed"))
            .env("GEM_PATH", installed_gem_path(&root)),
        &root,
        "executable-examples",
    );
    let dir = root.join("protocol-types");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Steepfile"),
        "target :consumer do\n  library 'ruby-native-gate'\n  check 'consumer.rb'\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("consumer.rb"),
        include_str!("../src/ruby_sdk/tests/protocol_types.rb"),
    )
    .unwrap();
    checked(
        ruby_command()
            .arg(tools().join("bin/steep"))
            .args(["check", "--jobs=1"])
            .env(
                "GEM_PATH",
                format!("{}:{}", tools().display(), installed_gem_path(&root)),
            )
            .current_dir(&dir),
        &root,
        "protocol-types-positive",
    );
    for (i,source) in [
        "RubyNativeGate::Bytes.new(nil)",
        "RubyNativeGate::Models::UploadRequest.new(metadata: RubyNativeGate::Models::UploadRequestMetadata.new(title: 'x'))",
        "RubyNativeGate::Models::UploadRequest.new(file: 'not-bytes', metadata: RubyNativeGate::Models::UploadRequestMetadata.new(title: 'x'))",
        "RubyNativeGate::Models::UploadRequest.new(file: nil, metadata: RubyNativeGate::Models::UploadRequestMetadata.new(title: 'x'))",
        "RubyNativeGate::Client.new.security_probe(security: 'automatic')",
        "RubyNativeGate::BasicCredential.new(username: 1, password: 'p')",
        "RubyNativeGate::Client.new.send_lines(body: ['wrong'])",
        "RubyNativeGate::Client.new.events.data.next.data = true",
    ].iter().enumerate(){
        std::fs::write(dir.join("consumer.rb"),source).unwrap();
        let result=ruby_command().arg(tools().join("bin/steep")).args(["check","--jobs=1"]).env("GEM_PATH",format!("{}:{}",tools().display(),installed_gem_path(&root))).current_dir(&dir).output().unwrap();
        let text=format!("{}{}",String::from_utf8_lossy(&result.stdout),String::from_utf8_lossy(&result.stderr));std::fs::write(root.join(format!("protocol-types-negative-{i}.log")),&text).unwrap();
        assert!(!result.status.success()&&text.contains("Ruby::")&&!text.contains("UnknownConstant"),"negative consumer accepted or unavailable: {source}\n{text}");
    }
}

#[test]
#[ignore = "requires tracked OpenRouter source and native Ruby; explicit binary compatibility profile"]
fn native_additional_openrouter_binary_and_delete_operations() {
    let web =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"));
    let c = load(&web.join("projects/docs/openapi/openapi.yaml"));
    let ids = [
        "downloadContainerFileContent",
        "downloadFileContent",
        "deleteKeys",
    ];
    let selected = c
        .operations()
        .filter(|o| ids.contains(&o.operation_id().unwrap_or_default()))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    assert!(
        plan_sdk(c.clone(), &selected, RubyConfig::default())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "http-binary-legacy-marker")
    );
    let plan = plan_sdk(
        c,
        &selected,
        RubyConfig {
            legacy_binary_strings: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(
        plan.protocol()
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-compatibility-profile")
    );
    let root = native_package(&plan, "openrouter-protocol");
    consumer(
        &root,
        "openrouter_protocol.rb",
        include_str!("../src/ruby_sdk/tests/openrouter_protocol.rb"),
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed(&root).join("sig"))
            .arg("validate"),
        &root,
        "rbs-validate",
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/yard"))
            .arg("doc")
            .current_dir(installed(&root)),
        &root,
        "yard-doc",
    );
    assert!(
        installed(&root)
            .join("doc/RubyNativeGate/Client.html")
            .is_file()
    );
}

#[test]
#[ignore = "requires native Ruby; OpenAPI 3.0 normalized codecs and byte contexts"]
fn native_oas30_nullable_reference_siblings_and_binary() {
    let doc = json!({"openapi":"3.0.4","info":{"title":"Ruby 3.0 witness","version":"1"},"servers":[{"url":"https://example.test"}],"components":{"schemas":{
        "Old":{"type":"object","required":["id","note"],"properties":{"id":{"type":"string"},"note":{"type":"string","nullable":true},"optional":{"type":"string","default":"receiver-default"}},"additionalProperties":false},
        "Alias":{"$ref":"#/components/schemas/Old","type":"integer","readOnly":true}
    }},"paths":{
        "/echo":{"post":{"operationId":"echoOld","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Alias"}}}},"responses":{"200":{"description":"old","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Old"}}}}}}},
        "/bytes":{"get":{"operationId":"oldBytes","responses":{"200":{"description":"bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}
    }});
    let base = root().join("target/sdk-ruby-protocol-fixtures");
    std::fs::create_dir_all(&base).unwrap();
    let path = tempfile::Builder::new()
        .prefix("oas30-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .join("api.json");
    std::fs::write(&path, doc.to_string()).unwrap();
    let c = load(&path);
    let plan = plan_sdk(c.clone(), &selected(&c), RubyConfig::default()).unwrap();
    assert!(
        plan.protocol()
            .diagnostics()
            .iter()
            .all(|d| d.code() != "http-compatibility-profile")
    );
    let root = native_package(&plan, "oas30");
    consumer(
        &root,
        "oas30.rb",
        r#"
require 'ruby_native_gate'
include RubyNativeGate
transport = Object.new
seen = []
transport.define_singleton_method(:exchange) do |request:, context:, &block|
  seen << request
  content, bytes = request.operation_id == 'oldBytes' ? ['application/octet-stream', "\x00\xff".b] : ['application/json', '{"id":"old","note":null}']
  block.call(WireResponse.new(status: 200, headers: {'Content-Type' => content}, body: bytes))
end
Client.open(transport: transport) do |client|
  input = Models::Old.new(id: 'old', note: nil)
  raise 'default inserted' unless input.optional.equal?(UNSET)
  raise 'nullable lost' unless client.echo_old(body: input).data.note.nil?
  raise 'byte marker was not normalized' unless client.old_bytes.data.data == "\x00\xff".b
end
raise 'wire presence changed' unless seen.first.body == '{"id":"old","note":null}'
puts 'OAS 3.0 nullable, reference-only siblings, omitted defaults and native binary passed'
"#,
    );
    checked(
        ruby_command()
            .arg(tools().join("bin/rbs"))
            .arg("-I")
            .arg(installed(&root).join("sig"))
            .arg("validate"),
        &root,
        "rbs-validate",
    );
}
