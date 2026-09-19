//! Maintained source fixtures -> checked compile_v3 -> installed native runtime.
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};
use suspect_codegen::rust_validation::emit;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.test/native.json";
static NATIVE: Mutex<()> = Mutex::new(());
fn id(name: &str) -> SchemaId {
    SchemaId::new(Uri::parse(ENTRY).unwrap(), Default::default())
        .child("components")
        .child("schemas")
        .child(name)
}
fn supplied(schemas: Value, extra: Vec<(&str, Value)>) -> Arc<Contract> {
    let mut docs = vec![(
        ENTRY,
        json!({"openapi":"3.2.0","info":{"title":"Rust v3","version":"1"},"paths":{},"components":{"schemas":schemas}}),
    )];
    docs.extend(extra);
    let provider = Arc::new(
        DocumentProvider::new(docs.into_iter().map(|(uri, value)| {
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
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap())
}
fn compile(contract: Arc<Contract>, roots: &[SchemaId], config: Config) -> OwnedProgram {
    let schema = OwnedCompiler::new(config)
        .compile_v3(contract.clone(), roots)
        .unwrap();
    let p = schema.program();
    p.check().unwrap();
    assert_eq!(
        (p.version, p.profile),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    for node in &p.nodes {
        assert!(
            contract
                .schemas()
                .any(|s| s.id().document().as_str() == node.source.document
                    && s.id().pointer() == node.source.pointer)
        );
    }
    p
}
fn checked(command: &mut Command, root: &Path) {
    let out = command.output().unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        log,
        "{command:?}\nstatus: {}\n{}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .unwrap();
    assert!(
        out.status.success(),
        "retained v3 attempt {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut c = Command::new("cargo");
    c.args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        c.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    c
}
fn target() -> PathBuf {
    std::env::var_os("SUSPECT_RUST_V3_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-validation-v3-msrv"
                } else {
                    "../../target/native-rust-validation-v3"
                },
            )
        })
}
fn native(programs: &[OwnedProgram], assertions: &str, private: &str) {
    let _gate = NATIVE.lock().unwrap_or_else(|e| e.into_inner());
    let root = tempfile::Builder::new()
        .prefix("rust-v3-validation-")
        .tempdir()
        .unwrap()
        .keep();
    let package = root.join("package");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(
        package.join("src/support.rs"),
        include_str!("../src/rust_models/runtime.rs"),
    )
    .unwrap();
    std::fs::write(
        package.join("src/json.rs"),
        include_str!("../src/rust_codecs/json_runtime.rs"),
    )
    .unwrap();
    let mut lib = String::from(
        "#![forbid(unsafe_code)]\nmod support;pub mod json;pub use support::{ExtraFieldError,JsonInteger,JsonNonNullValue,JsonNumber,JsonValue,Never,Nullable,NumberError,Presence};\n",
    );
    for (index, p) in programs.iter().enumerate() {
        for file in emit(p).unwrap() {
            let relative = file.path.strip_prefix("rust/src/").unwrap();
            let path = if relative == "validation.rs" {
                format!("p{index}.rs")
            } else {
                format!("p{index}/{}", relative.strip_prefix("validation/").unwrap())
            };
            let path = package.join("src").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file.content).unwrap();
        }
        lib.push_str(&format!("pub mod p{index};\n"));
    }
    lib.push_str(private);
    std::fs::write(package.join("src/lib.rs"), lib).unwrap();
    std::fs::write(package.join("Cargo.toml"),"[package]\nname=\"native-v3-validation\"\nversion=\"0.0.0\"\nedition=\"2024\"\nrust-version=\"1.88\"\npublish=false\n[workspace]\n").unwrap();
    let target = target();
    let manifest = package.join("Cargo.toml");
    checked(&mut cargo("test", &manifest, &target), &root);
    checked(cargo("doc", &manifest, &target).arg("--no-deps"), &root);
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &root,
    );
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join("package/native-v3-validation-0.0.0.crate");
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &root,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"native-v3-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"native-v3-validation\",path=\"vendor/native-v3-validation-0.0.0\"}\n").unwrap();
    std::fs::write(
        consumer.join("src/main.rs"),
        format!("fn main(){{{assertions}}}"),
    )
    .unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()));
    checked(
        &mut cargo(
            "run",
            &consumer.join("Cargo.toml"),
            &target.join("installed").join(digest),
        ),
        &root,
    );
    eprintln!("native v3 retained {}", root.display());
}
fn assertion(
    module: usize,
    p: &OwnedProgram,
    root: &SchemaId,
    input: &str,
    kind: &str,
    location: (Option<&str>, Option<&str>),
    label: &str,
) -> String {
    let (at, path) = location;
    let selected = p
        .roots
        .iter()
        .find(|r| {
            r.source.document == root.document().as_str() && r.source.pointer == root.pointer()
        })
        .unwrap();
    let target = selected.target;
    format!("{{let value=sdk::json::parse_json({input:?},Default::default()).unwrap();let (kind,findings)=match sdk::p{module}::validate({target},&value){{sdk::p{module}::ValidationOutcome::Valid=>(\"Valid\",vec![]),sdk::p{module}::ValidationOutcome::Invalid(f)=>(\"Invalid\",f),sdk::p{module}::ValidationOutcome::EvaluationFailure(f)=>(\"EvaluationFailure\",vec![f])}};assert_eq!(kind,{kind:?},\"{label}: {{findings:?}}\");{}{} }}\n",at.map(|at|format!("assert!(findings.iter().any(|f|f.pointer=={at:?}),\"{label}: {{findings:?}}\");")).unwrap_or_default(),path.map(|path|format!("assert!(findings.iter().any(|f|f.instance_path=={path:?}),\"{label}: {{findings:?}}\");")).unwrap_or_default())
}

#[test]
fn v3_checked_emission_rejects_malformed_resources_and_old_envelopes() {
    let root = id("Outer");
    let p = compile(
        supplied(
            json!({"Base":{"$id":"urn:base","$dynamicAnchor":"slot","type":"string"},"Outer":{"$id":"urn:outer","$defs":{"Slot":{"$dynamicAnchor":"slot","type":"integer"}},"$dynamicRef":"urn:base#slot"}}),
            vec![],
        ),
        &[root],
        Config::default(),
    );
    assert!(emit(&p).is_ok());
    for (v, f) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut wrong = p.clone();
        wrong.version = v;
        wrong.profile = f;
        assert!(emit(&wrong).is_err());
        wrong.resource_context = None;
        assert!(emit(&wrong).is_err());
    }
    for change in 0..6 {
        let mut wrong = p.clone();
        let c = wrong.resource_context.as_mut().unwrap();
        match change {
            0 => {
                c.node_scopes.pop();
            }
            1 => c.node_scopes[0].0 = usize::MAX,
            2 => c.node_scopes[0].2 = "urn:unrelated".into(),
            3 => c.resources[0].aliases.clear(),
            4 => {
                let r = c
                    .resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap();
                r.dynamic_anchors[0].2 = usize::MAX;
            }
            _ => {
                let check = wrong
                    .nodes
                    .iter_mut()
                    .flat_map(|n| &mut n.checks)
                    .find(|c| matches!(c.instruction, ProgramInstruction::DynamicRef { .. }))
                    .unwrap();
                if let ProgramInstruction::DynamicRef {
                    initial_resource, ..
                } = &mut check.instruction
                {
                    *initial_resource = usize::MAX;
                }
            }
        }
        assert!(emit(&wrong).is_err(), "mutation {change}");
    }
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn installed_rust_v3_executes_all_official_dynamic_ref_source_fixtures() {
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut programs = Vec::new();
    let mut assertions = String::new();
    let mut count = 0;
    for (group_index, group) in groups.iter().enumerate() {
        let document = "https://physical.test/official-schema.json";
        let c=supplied(json!({"Use":{"$ref":document}}),vec![
            (document,group["schema"].clone()),
            ("http://localhost:1234/draft2020-12/tree.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json")).unwrap()),
        ]);
        let root = SchemaId::new(Uri::parse(document).unwrap(), Default::default());
        let p = compile(c, std::slice::from_ref(&root), Config::default());
        for (case_index, case) in group["tests"].as_array().unwrap().iter().enumerate() {
            count += 1;
            assertions.push_str(&assertion(
                programs.len(),
                &p,
                &root,
                &case["data"].to_string(),
                if case["valid"].as_bool().unwrap() {
                    "Valid"
                } else {
                    "Invalid"
                },
                (None, None),
                &format!("official-{group_index}-{case_index}"),
            ));
        }
        programs.push(p);
    }
    assert_eq!(count, 44);
    for text in [
        include_str!(
            "../../suspect-schema/tests/conformance/draft2020-12/unevaluatedProperties.json"
        ),
        include_str!("../../suspect-schema/tests/conformance/draft2020-12/unevaluatedItems.json"),
    ] {
        let groups: Vec<Value> = serde_json::from_str(text).unwrap();
        for group in groups {
            if !matches!(
                group["description"].as_str(),
                Some(
                    "unevaluatedProperties with $dynamicRef" | "unevaluatedItems with $dynamicRef"
                )
            ) {
                continue;
            }
            let document = "https://physical.test/unevaluated.json";
            let root = SchemaId::new(Uri::parse(document).unwrap(), Default::default());
            let p = compile(
                supplied(
                    json!({"Use":{"$ref":document}}),
                    vec![(document, group["schema"].clone())],
                ),
                std::slice::from_ref(&root),
                Config::default(),
            );
            for case in group["tests"].as_array().unwrap() {
                count += 1;
                assertions.push_str(&assertion(
                    programs.len(),
                    &p,
                    &root,
                    &case["data"].to_string(),
                    if case["valid"].as_bool().unwrap() {
                        "Valid"
                    } else {
                        "Invalid"
                    },
                    (None, None),
                    &format!("dynamic-unevaluated-{count}"),
                ));
            }
            programs.push(p);
        }
    }
    assert_eq!(count, 48);
    native(&programs, &assertions, "");
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn installed_rust_v3_scope_cycles_fallbacks_and_work_failures_are_independent() {
    let c = supplied(
        json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#node"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:aaa","$dynamicAnchor":"node","not":{}},
            "String":{"$id":"urn:string","type":"string","$defs":{"Slot":{"$anchor":"static","$dynamicAnchor":"slot","type":"string"}}},
            "Variants":{"$id":"urn:variants","$defs":{"Slot":{"$dynamicAnchor":"slot","type":"integer"}},"properties":{"dynamic":{"$dynamicRef":"urn:string#slot"},"pointer":{"$dynamicRef":"urn:string#/$defs/Slot"},"plain":{"$dynamicRef":"urn:string#static"},"empty":{"$dynamicRef":"urn:string#"},"static":{"$ref":"urn:string#slot"}}},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"slot","not":{}},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:string#slot"}]},
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},
            "Reenter":{"$id":"urn:reenter","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:context"}},
            "Context":{"$id":"urn:context","$defs":{"Binding":{"$dynamicAnchor":"flag"}},"$ref":"urn:reenter"}
        }),
        vec![],
    );
    let roots = [
        id("Outer"),
        id("Unentered"),
        id("Variants"),
        id("Trial"),
        id("Reenter"),
    ];
    let p = compile(c, &roots, Config::default());
    let mut assertions = String::new();
    for (root, input, kind) in [
        (
            "Outer",
            r#"{"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}"#,
            "Valid",
        ),
        (
            "Outer",
            r#"{"outer":true,"middle":true,"children":[{"middle":true}]}"#,
            "Invalid",
        ),
        (
            "Variants",
            r#"{"dynamic":1,"pointer":"s","plain":"s","empty":"s","static":"s"}"#,
            "Valid",
        ),
        ("Variants", r#"{"dynamic":"s"}"#, "Invalid"),
        ("Variants", r#"{"pointer":1}"#, "Invalid"),
        ("Variants", r#"{"plain":1}"#, "Invalid"),
        ("Variants", r#"{"empty":1}"#, "Invalid"),
        ("Trial", r#""ok""#, "Valid"),
        ("Trial", "7", "Invalid"),
        ("Reenter", "7", "Valid"),
    ] {
        assertions.push_str(&assertion(
            0,
            &p,
            &id(root),
            input,
            kind,
            (None, None),
            root,
        ));
    }
    let mut programs = vec![p];
    let outer = "https://physical.test/nested.json";
    let root = SchemaId::new(Uri::parse(outer).unwrap(), Default::default())
        .child("$defs")
        .child("Start");
    let p = compile(
        supplied(
            json!({"Use":{"$ref":format!("{outer}#/$defs/Start")},"Base":{"$id":"urn:base","$dynamicAnchor":"slot","type":"string"}}),
            vec![(
                outer,
                json!({"$id":"urn:nested","type":"boolean","$defs":{"Binding":{"$dynamicAnchor":"slot","type":"integer"},"Start":{"$dynamicRef":"urn:base#slot"}}}),
            )],
        ),
        std::slice::from_ref(&root),
        Config::default(),
    );
    assert!(
        p.nodes
            .iter()
            .all(|n| n.source.document != outer || !n.source.pointer.is_empty())
    );
    assertions.push_str(&assertion(
        1,
        &p,
        &root,
        "7",
        "Valid",
        (None, None),
        "nested entry enters resource only",
    ));
    assertions.push_str(&assertion(
        1,
        &p,
        &root,
        r#""s""#,
        "Invalid",
        (Some("/$defs/Binding/type"), Some("")),
        "nested physical finding",
    ));
    programs.push(p);
    for limit in [6, 7] {
        let root = id("Use");
        let p = compile(
            supplied(
                json!({"Use":{"$id":"urn:use","$defs":{"Binding":{"$dynamicAnchor":"slot","type":"integer"}},"$dynamicRef":"urn:base#slot"},"Base":{"$id":"urn:base","$dynamicAnchor":"slot","type":"string"}}),
                vec![],
            ),
            std::slice::from_ref(&root),
            Config {
                max_evaluation_steps: limit,
                ..Default::default()
            },
        );
        assertions.push_str(&assertion(
            programs.len(),
            &p,
            &root,
            "7",
            if limit == 7 {
                "Valid"
            } else {
                "EvaluationFailure"
            },
            (None, None),
            "resource scan charging",
        ));
        programs.push(p);
    }
    for use_site in [
        json!({"if":{"$dynamicRef":"urn:number#slot"},"then":true,"else":true}),
        json!({"anyOf":[true,{"$dynamicRef":"urn:number#slot"}]}),
        json!({"not":{"$dynamicRef":"urn:number#slot"}}),
    ] {
        let root = id("Use");
        let p = compile(
            supplied(
                json!({"Use":use_site,"Number":{"$id":"urn:number","$dynamicAnchor":"slot","maximum":0}}),
                vec![],
            ),
            std::slice::from_ref(&root),
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Default::default()
            },
        );
        assertions.push_str(&assertion(
            programs.len(),
            &p,
            &root,
            "12345",
            "EvaluationFailure",
            (Some("/components/schemas/Number/maximum"), Some("")),
            "dynamic failure cannot become mismatch",
        ));
        programs.push(p);
    }
    native(&programs, &assertions, "");
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn installed_rust_v3_default_depth_scope_restore_and_annotation_isolation() {
    let mut programs = Vec::new();
    let mut assertions = String::new();
    // Independent 512/513 boundary with a new distinct indexed resource at
    // every node. The instance is shallow, so JSON/conversion limits are inert.
    for count in [512, 513] {
        let schemas = (0..count)
            .map(|index| {
                let uri = format!("urn:rust-v3-depth:{index}");
                (
                    format!("N{index}"),
                    if index + 1 == count {
                        json!({"$id":uri})
                    } else {
                        json!({"$id":uri,"$ref":format!("urn:rust-v3-depth:{}",index+1)})
                    },
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let root = id("N0");
        let p = compile(
            supplied(Value::Object(schemas), vec![]),
            std::slice::from_ref(&root),
            Config::default(),
        );
        assert_eq!(p.nodes.len(), count);
        let check = assertion(
            programs.len(),
            &p,
            &root,
            "null",
            if count == 512 {
                "Valid"
            } else {
                "EvaluationFailure"
            },
            (
                if count == 513 {
                    Some("/components/schemas/N512")
                } else {
                    None
                },
                None,
            ),
            "default-depth-boundary",
        );
        assertions.push_str(&format!("std::thread::Builder::new().stack_size(2*1024*1024).spawn(||{{{check}}}).unwrap().join().unwrap();\n"));
        programs.push(p);
    }
    let c = supplied(
        json!({
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"slot","type":"string"},
            "Use":{"$dynamicRef":"urn:fallback#slot"},
            "Fail":{"$id":"urn:poison-failure","$defs":{"Binding":{"$dynamicAnchor":"slot","type":"boolean"}},"type":"string","maximum":0},
            "Invalid":{"$id":"urn:poison-invalid","$defs":{"Binding":{"$dynamicAnchor":"slot","type":"boolean"}},"type":"integer"},
            "Valid":{"$id":"urn:poison-valid","$defs":{"Binding":{"$dynamicAnchor":"slot","type":"boolean"}},"type":"integer"},
            "Cycle":{"$id":"urn:cycle","$dynamicAnchor":"slot","$dynamicRef":"#slot"},
            "PairA":{"$id":"urn:pair-a","$ref":"urn:pair-b"},"PairB":{"$id":"urn:pair-b","$ref":"urn:pair-a"},
            "Closed":{"$id":"urn:closed","$dynamicAnchor":"closed","unevaluatedProperties":false},
            "FreshTarget":{"if":{"properties":{"seen":true}},"then":{"$dynamicRef":"urn:closed#closed"}},
            "Marks":{"$id":"urn:marks","$dynamicAnchor":"marks","properties":{"allowed":true}},
            "Propagate":{"$dynamicRef":"urn:marks#marks","unevaluatedProperties":false},
            "A":{"$id":"urn:order-a","$defs":{"Binding":{"$dynamicAnchor":"slot","const":"a"},"Probe":{"$ref":"urn:common"}},"$ref":"urn:order-b#/$defs/Probe"},
            "B":{"$id":"urn:order-b","$defs":{"Binding":{"$dynamicAnchor":"slot","const":"b"},"Probe":{"$ref":"urn:common"}},"$ref":"urn:order-a#/$defs/Probe"},
            "Common":{"$id":"urn:common","$dynamicRef":"urn:fallback#slot"}
        }),
        vec![],
    );
    let roots = [
        "Use",
        "Fail",
        "Invalid",
        "Valid",
        "Cycle",
        "PairA",
        "FreshTarget",
        "Propagate",
        "A",
        "B",
    ]
    .map(id);
    let p = compile(
        c,
        &roots,
        Config {
            max_number_bytes: 3,
            max_errors: 1,
            max_equality_steps: 10,
            ..Default::default()
        },
    );
    let module = programs.len();
    for (name, input, outcome, pointer, path) in [
        (
            "Cycle",
            "null",
            "EvaluationFailure",
            Some("/components/schemas/Cycle"),
            Some(""),
        ),
        (
            "PairA",
            "null",
            "EvaluationFailure",
            Some("/components/schemas/PairB"),
            Some(""),
        ),
        (
            "FreshTarget",
            r#"{"seen":1}"#,
            "Invalid",
            Some("/components/schemas/Closed/unevaluatedProperties"),
            None,
        ),
        ("Propagate", r#"{"allowed":1}"#, "Valid", None, None),
        (
            "Propagate",
            r#"{"extra":1}"#,
            "Invalid",
            Some("/components/schemas/Propagate/unevaluatedProperties"),
            None,
        ),
        ("A", r#""a""#, "Valid", None, None),
        ("A", r#""b""#, "Invalid", None, None),
        ("B", r#""b""#, "Valid", None, None),
        ("B", r#""a""#, "Invalid", None, None),
    ] {
        assertions.push_str(&assertion(
            module,
            &p,
            &id(name),
            input,
            outcome,
            (pointer, path),
            name,
        ));
    }
    let index = |name: &str| {
        p.roots
            .iter()
            .find(|r| r.source.pointer == id(name).pointer())
            .unwrap()
            .target
    };
    let (valid, invalid, fail, normal, cycle, a, b) = (
        index("Valid"),
        index("Invalid"),
        index("Fail"),
        index("Use"),
        index("Cycle"),
        index("A"),
        index("B"),
    );
    let private = format!(
        r#"
    #[test] fn sessions_restore_resources_after_valid_invalid_and_failure_returns(){{
        use p{module}::{{ValidationOutcome as O,ValidationSession}};
        let mut session=ValidationSession::new();let good=json::parse_json("\"ok\"",Default::default()).unwrap();
        let number=json::parse_json("1",Default::default()).unwrap();let huge=json::parse_json("12345",Default::default()).unwrap();
        assert!(matches!(session.validate_at({valid},&number,"/valid"),O::Valid));
        assert!(matches!(session.validate_at({normal},&good,"/after-valid"),O::Valid));
        assert!(matches!(session.validate_at({invalid},&good,"/invalid"),O::Invalid(_)));
        assert!(matches!(session.validate_at({normal},&good,"/after-invalid"),O::Valid));
        let O::EvaluationFailure(finding)=session.validate_at({fail},&huge,"/failure") else{{panic!("a capped ordinary finding must not hide numeric failure")}};
        assert_eq!(finding.pointer,"/components/schemas/Fail/maximum");assert_eq!(finding.instance_path,"/failure");
        assert!(matches!(session.validate_at({normal},&good,"/after-failure"),O::Valid));
        assert!(matches!(session.validate_at({cycle},&good,"/cycle"),O::EvaluationFailure(_)));
        assert!(matches!(session.validate_at({normal},&good,"/after-cycle"),O::Valid));
        let value=json::parse_json("\"a\"",Default::default()).unwrap();
        assert!(matches!(session.validate_at({a},&value,"/ab"),O::Valid));
        assert!(matches!(session.validate_at({b},&value,"/ba"),O::Invalid(_)));
        assert!(matches!(session.validate_at({a},&value,"/ab-again"),O::Valid));
    }}
    "#
    );
    programs.push(p);
    // The first scanned resource has no bindings. The next resource scans an
    // unrelated binding before the matching one; each visit spends one step.
    for limit in [11, 12] {
        let c = supplied(
            json!({
                "Root":{"$id":"urn:scan-root","$ref":"urn:scan-use"},
                "Use":{"$id":"urn:scan-use","$defs":{"A":{"$dynamicAnchor":"alpha","type":"boolean"},"Z":{"$dynamicAnchor":"slot","type":"integer"}},"$dynamicRef":"urn:scan-base#slot"},
                "Base":{"$id":"urn:scan-base","$dynamicAnchor":"slot","type":"string"}
            }),
            vec![],
        );
        let root = id("Root");
        let p = compile(
            c,
            std::slice::from_ref(&root),
            Config {
                max_evaluation_steps: limit,
                ..Default::default()
            },
        );
        assert_eq!(
            p.resource_context
                .as_ref()
                .unwrap()
                .resources
                .iter()
                .find(|r| r.canonical_uri == "urn:scan-use")
                .unwrap()
                .dynamic_anchors
                .len(),
            2
        );
        assertions.push_str(&assertion(
            programs.len(),
            &p,
            &root,
            "1",
            if limit == 12 {
                "Valid"
            } else {
                "EvaluationFailure"
            },
            (None, None),
            "nonmatching-resource-and-binding-work",
        ));
        programs.push(p);
    }
    native(&programs, &assertions, &private);
}
