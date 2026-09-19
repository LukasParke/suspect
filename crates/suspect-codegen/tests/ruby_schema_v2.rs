//! Source Contract -> compile_v2 -> installed native Ruby validation, followed by
//! real SDK model/codec/operation admission. No target witness file is an input.
#![cfg(feature = "ruby-sdk")]

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::ruby_sdk::{self, PackageConfig, RubyConfig};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn source(schema: Value) -> (Arc<Contract>, SchemaId) {
    let base = root().join("target/sdk-ruby-schema-v2-sources");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::Builder::new()
        .prefix("source-")
        .tempdir_in(base)
        .unwrap()
        .keep();
    let path = dir.join("api.json");
    std::fs::write(&path,json!({
        "openapi":"3.1.2","info":{"title":"Ruby scoped schema","version":"1"},
        "servers":[{"url":"https://schema.example.test"}],"components":{"schemas":{"Root":schema}},
        "paths":{"/check":{"post":{
            "operationId":"checkValue",
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}},
            "responses":{"200":{"description":"Checked","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}}}
        }}}
    }).to_string()).unwrap();
    let ws = Arc::new(WorkspaceBuilder::new().root(&dir).build().unwrap());
    let c = Arc::new(Contract::from_workspace(&ws, &Uri::from_path(&path).unwrap()).unwrap());
    let id = SchemaId::new(c.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (c, id)
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
    assert!(
        !text.contains("[error]"),
        "native tool reported an error: {text}"
    );
}
fn package(plan: &ruby_sdk::SdkPlan, label: &str) -> PathBuf {
    let base = root().join("target/sdk-ruby-schema-v2-native");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep();
    let p = PackageConfig {
        name: "ruby-schema-v2-gate".into(),
        version: "0.1.0".into(),
        require_name: "ruby_schema_v2".into(),
        namespace: "RubySchemaV2".into(),
    };
    for f in ruby_sdk::emit_sdk(plan, &p).unwrap() {
        let target = dir.join(f.path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, f.content).unwrap();
    }
    checked(ruby().arg("-v"), &dir, "ruby-version");
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["build", "ruby-schema-v2-gate.gemspec"])
            .current_dir(dir.join("ruby")),
        &dir,
        "gem-build",
    );
    checked(
        ruby()
            .arg(ruby_home().join("bin/gem"))
            .args(["install", "--local", "--no-document"])
            .arg(dir.join("ruby/ruby-schema-v2-gate-0.1.0.gem"))
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
    dir
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

fn case(
    id: &str,
    schema: Value,
    instance: &str,
    expected: &str,
    config: Config,
    at: Option<&str>,
    path: Option<&str>,
) -> Value {
    let (c, root) = source(schema);
    let compiled = OwnedCompiler::new(config)
        .compile_v2(c, std::slice::from_ref(&root))
        .unwrap();
    let program = compiled.program();
    program.check().unwrap();
    json!({"id":id,"program":program,"instanceJson":instance,"expected":expected,"source":at,"instancePath":path})
}

#[test]
#[ignore = "requires native Ruby; compiles the maintained 32 source cases through compile_v2 and installs the Ruby runtime"]
fn source_driven_v2_instructions_scopes_budgets_and_program_guards() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../suspect-schema/tests/fixtures/owned-applicators-v2.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let entries = fixture["cases"].as_array().unwrap();
    assert_eq!(entries.len(), 32);
    let mut cases = Vec::new();
    for c in entries {
        let mut limits = Config::default();
        if let Some(n) = c["limits"]["maxNumberBytes"].as_u64() {
            limits.max_number_bytes = n as usize;
        }
        if let Some(n) = c["limits"]["maxEvaluationSteps"].as_u64() {
            limits.max_evaluation_steps = n as usize;
        }
        cases.push(case(
            c["id"].as_str().unwrap(),
            serde_json::from_str(c["schemaJson"].as_str().unwrap()).unwrap(),
            c["instanceJson"].as_str().unwrap(),
            c["expected"].as_str().unwrap(),
            limits,
            c["source"].as_str(),
            None,
        ));
    }
    let property = json!({"type":"object","properties":{"a":true},"unevaluatedProperties":false});
    cases.push(case(
        "merge-exact-nine",
        property.clone(),
        "{\"a\":1}",
        "Valid",
        Config {
            max_evaluation_steps: 9,
            ..Default::default()
        },
        None,
        None,
    ));
    cases.push(case(
        "merge-eight-is-incomplete",
        property,
        "{\"a\":1}",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 8,
            ..Default::default()
        },
        Some("/components/schemas/Root/unevaluatedProperties"),
        Some(""),
    ));
    let union = json!({"anyOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false});
    cases.push(case(
        "duplicate-merges-cost-21",
        union.clone(),
        "{\"a\":1}",
        "Valid",
        Config {
            max_evaluation_steps: 21,
            ..Default::default()
        },
        None,
        None,
    ));
    cases.push(case(
        "duplicate-merge-not-free",
        union,
        "{\"a\":1}",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 17,
            ..Default::default()
        },
        Some("/components/schemas/Root/anyOf"),
        Some(""),
    ));
    cases.push(case(
        "names-real-member-path",
        json!({"propertyNames":{"maxLength":0}}),
        "{\"a/b~\":null}",
        "Invalid",
        Config::default(),
        Some("/components/schemas/Root/propertyNames/maxLength"),
        Some("/a~1b~0"),
    ));
    cases.push(case(
        "property-name-equality-failure",
        json!({"propertyNames":{"enum":["a"]}}),
        "{\"a\":null}",
        "EvaluationFailure",
        Config {
            max_equality_steps: 0,
            ..Default::default()
        },
        Some("/components/schemas/Root/propertyNames/enum"),
        Some("/a"),
    ));
    cases.push(case(
        "contains-implicit-is-not-source-number",
        json!({"contains":true}),
        "[12345]",
        "Valid",
        Config {
            max_number_bytes: 0,
            ..Default::default()
        },
        None,
        None,
    ));
    cases.push(case(
        "pattern-failure-still-excludes-name",
        json!({"patternProperties":{"^x":false},"additionalProperties":{"type":"integer"}}),
        "{\"x\":12345}",
        "Invalid",
        Config {
            max_number_bytes: 3,
            ..Default::default()
        },
        Some("/components/schemas/Root/patternProperties/^x"),
        Some("/x"),
    ));
    cases.push(case(
        "pattern-work-shared",
        json!({"patternProperties":{"^x+$":true}}),
        "{\"xxxxxxxxxxxxxxxx\":true}",
        "EvaluationFailure",
        Config {
            max_evaluation_steps: 8,
            ..Default::default()
        },
        Some("/components/schemas/Root/patternProperties"),
        Some(""),
    ));
    cases.push(case(
        "condition-scope-not-seeded-into-then",
        json!({"if":{"properties":{"a":true}},"then":{"unevaluatedProperties":false}}),
        "{\"a\":1}",
        "Invalid",
        Config::default(),
        Some("/components/schemas/Root/then/unevaluatedProperties"),
        Some("/a"),
    ));
    cases.push(case(
        "recursive-property-name-stable-identity",
        json!({"propertyNames":{"$ref":"#/components/schemas/Root/propertyNames"}}),
        "{\"a\":1}",
        "EvaluationFailure",
        Config::default(),
        Some("/components/schemas/Root/propertyNames"),
        Some("/a"),
    ));
    cases.push(case(
        "noninvertible-dependent-cycle",
        json!({"not":{"dependentSchemas":{"x":{"$ref":"#/components/schemas/Root"}}}}),
        "{\"x\":null}",
        "EvaluationFailure",
        Config::default(),
        Some("/components/schemas/Root"),
        Some(""),
    ));
    let (c, id) = source(json!({"type":"integer","minimum":0}));
    let compiler = OwnedCompiler::new(Config::default());
    let old = compiler
        .compile(c.clone(), std::slice::from_ref(&id))
        .unwrap()
        .program();
    let new = compiler
        .compile_v2(c, std::slice::from_ref(&id))
        .unwrap()
        .program();
    assert_eq!(
        serde_json::to_vec(&old).unwrap(),
        serde_json::to_vec(&new).unwrap()
    );
    assert_eq!(new.version, OwnedProgram::V1_VERSION);
    cases.push(json!({"id":"unchanged-v1","program":new,"instanceJson":"1.0","expected":"Valid"}));
    let (c, _) = source(json!(true));
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let vehicle = ruby_sdk::plan_sdk(c, &selected, RubyConfig::default()).unwrap();
    assert_eq!(vehicle.program().version, OwnedProgram::V1_VERSION);
    let dir = package(&vehicle, "vectors-");
    std::fs::write(
        dir.join("cases.json"),
        serde_json::to_string(&cases).unwrap(),
    )
    .unwrap();
    consumer(
        &dir,
        include_str!("../src/ruby_sdk/tests/schema_v2_vectors.rb"),
        "vectors",
    );
}

fn scoped_sdk() -> ruby_sdk::SdkPlan {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ruby_sdk/tests/schema_v2.openapi.json");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    ruby_sdk::plan_sdk(contract, &selected, RubyConfig::default()).unwrap()
}

#[test]
fn scoped_native_descriptors_retain_fields_patterns_and_real_program_identity() {
    let plan = scoped_sdk();
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    let symbol = plan
        .models()
        .symbols()
        .find(|s| s.name == "Settings")
        .unwrap();
    let ruby_sdk::ModelShape::Object { fields, extras, .. } = &symbol.shape else {
        panic!("named object model")
    };
    assert!(fields.iter().any(|f| f.wire_name == "name" && f.required));
    assert!(fields.iter().any(|f| f.wire_name == "note" && !f.required));
    let ruby_sdk::ExtraFields::Scoped {
        patterns,
        additional,
        unevaluated,
    } = extras
    else {
        panic!("pattern-aware exact JSON extras")
    };
    assert_eq!(patterns.len(), 2);
    assert!(additional.is_some() && unevaluated.is_some());
    assert!(
        patterns
            .iter()
            .all(|p| plan.contract().schema(&p.source).is_some())
    );
    assert_eq!(plan.examples().format(), "suspect-sdk-examples-v2");
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-declared-invalid"
                && d.source.pointer().ends_with("/examples/invalid/value"))
    );
    assert!(
        plan.examples()
            .operations()
            .iter()
            .all(|o| !o.entries.is_empty())
    );
    assert!(
        ruby_sdk::source_assets()
            .iter()
            .any(|(name, _)| *name == "ruby_sdk/validation_v2.rb")
    );

    let (c, _) = source(json!({"type":"integer","minimum":0}));
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plain = ruby_sdk::plan_sdk(c.clone(), &selected, RubyConfig::default()).unwrap();
    let ids = plain
        .program()
        .roots
        .iter()
        .map(|r| {
            c.schemas()
                .find(|s| {
                    s.id().document().as_str() == r.source.document
                        && s.id().pointer() == r.source.pointer
                })
                .unwrap()
                .id()
                .clone()
        })
        .collect::<Vec<_>>();
    let v1 = OwnedCompiler::new(RubyConfig::default().schema)
        .compile(c, &ids)
        .unwrap()
        .program();
    assert_eq!(plain.program().version, OwnedProgram::V1_VERSION);
    assert_eq!(
        serde_json::to_vec(plain.program()).unwrap(),
        serde_json::to_vec(&v1).unwrap()
    );
}

#[test]
#[ignore = "requires native Ruby, local gem install, RBS/Steep/YARD; actual v2 SDK calls and examples"]
fn installed_scoped_sdk_models_types_examples_and_wire() {
    let plan = scoped_sdk();
    let dir = package(&plan, "sdk-");
    consumer(
        &dir,
        include_str!("../src/ruby_sdk/tests/schema_v2_sdk.rb"),
        "sdk",
    );
    let installed = dir.join("installed/gems/ruby-schema-v2-gate-0.1.0");
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
    assert!(
        installed
            .join("doc/RubySchemaV2/Models/Settings.html")
            .is_file()
    );
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
        "target :consumer do\n  library 'ruby-schema-v2-gate'\n  check 'consumer.rb'\nend\n",
    )
    .unwrap();
    std::fs::write(
        types.join("consumer.rb"),
        include_str!("../src/ruby_sdk/tests/schema_v2_types.rb"),
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
    for(i,snippet)in [
        "RubySchemaV2::Models::Settings.new(name: 'x')",
        "RubySchemaV2::Models::Settings.new(mode: 'unknown', name: 'x')",
        "RubySchemaV2::Models::Settings.new(mode: 'simple', name: 1)",
        "RubySchemaV2::Models::Settings.new(mode: 'simple', name: 'x', note: false)",
        "RubySchemaV2::Models::Settings.new(mode: 'simple', name: 'x', extra_fields: {'x-rate' => 0.1})",
        "RubySchemaV2::Client.new.save_settings(body: {'mode' => 'simple', 'name' => 'x'})",
        "RubySchemaV2::Client.new.save_settings",
    ].iter().enumerate(){
        std::fs::write(types.join("consumer.rb"),snippet).unwrap();let output=ruby().arg(tools().join("bin/steep")).args(["check","--jobs=1"]).env("GEM_PATH",&gem_path).current_dir(&types).output().unwrap();
        let result=format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));std::fs::write(dir.join(format!("types-negative-{i}.log")),&result).unwrap();
        assert!(!output.status.success()&&result.contains("Ruby::")&&!result.contains("UnknownConstant"),"negative type witness did not run: {snippet}\n{result}");
    }
    for file in ruby_sdk::emit_sdk(
        &plan,
        &PackageConfig {
            name: "ruby-schema-v2-gate".into(),
            version: "0.1.0".into(),
            require_name: "ruby_schema_v2".into(),
            namespace: "RubySchemaV2".into(),
        },
    )
    .unwrap()
    {
        let relative = file.path.strip_prefix("ruby/").unwrap();
        if !relative.ends_with(".gemspec") {
            assert_eq!(
                std::fs::read(installed.join(relative)).unwrap(),
                file.content.as_bytes(),
                "installed artifact was modified: {relative}"
            );
        }
    }
}
