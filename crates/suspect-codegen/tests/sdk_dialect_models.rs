//! Native consumers of source-preserving OAS 3.0/modern schema views.
//! Oracle: OAS 3.0.4 Schema/Reference Objects and JSON Schema 2020-12 Core
//! §§8.2.3, 10.2.1 / Validation §6; not expectations inferred from emitted code.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    go_codecs, go_models, python_codecs, python_models, rust_codecs, rust_models, swift_sdk,
    typescript,
};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{
    Config, OwnedCompiler, OwnedOutcome, OwnedProgram, ProgramInstruction, ProgramType,
};
use suspect_source::Uri;

fn artifact_dir(label: &str) -> PathBuf {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    tempfile::Builder::new()
        .prefix(&format!("sdk-schema-{label}-"))
        .tempdir_in(target)
        .unwrap()
        .keep()
}
fn load(document: Value, external: Option<Value>) -> Arc<Contract> {
    let directory = artifact_dir("dialect-input");
    let entry = directory.join("api.json");
    std::fs::write(&entry, document.to_string()).unwrap();
    if let Some(external) = external {
        std::fs::write(directory.join("legacy.json"), external.to_string()).unwrap();
    }
    let workspace = Arc::new(WorkspaceBuilder::new().root(&directory).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap())
}
fn root(contract: &Contract, name: &str) -> SchemaId {
    SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child(name)
}
fn envelope(version: &str, schemas: Value) -> Value {
    json!({"openapi":version,"info":{"title":"Native dialects","version":"1"},
    "servers":[{"url":"https://example.test"}],"security":[{"token":[]}],
    "components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"}},"schemas":schemas},
    "paths":{"/record":{"post":{"operationId":"saveRecord",
        "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}},
        "responses":{"200":{"description":"Record","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}}}
    }}}})
}
fn fixture(version: &str) -> Arc<Contract> {
    let old = version.starts_with("3.0.");
    let nullable = if old {
        json!({"type":"string","nullable":true,"minLength":1})
    } else {
        json!({"type":["string","null"],"minLength":1})
    };
    let mut word = nullable.clone();
    word["enum"] = json!(["live"]);
    let mut reference = json!({"$ref":"#/components/schemas/Text"});
    let mut branch = json!({"$ref":"#/components/schemas/StringCase"});
    if old {
        // Every sibling must be ignored, including unsupported descendants,
        // invalid declarations, direction flags and null-only branch hints.
        reference = json!({"$ref":"#/components/schemas/Text","type":"integer","nullable":false,"enum":[false],"const":null,"readOnly":true,"required":[],
            "properties":{"Ignored":{"contains":true,"$ref":"#/missing"}},"description":"Ignored prose"});
        branch = json!({"$ref":"#/components/schemas/StringCase","type":"null","enum":[null],"allOf":[false]});
    }
    let small = if old {
        json!({"type":"integer","minimum":-1,"maximum":256,"exclusiveMinimum":true,"exclusiveMaximum":true})
    } else {
        json!({"type":"integer","exclusiveMinimum":-1,"exclusiveMaximum":256})
    };
    load(
        envelope(
            version,
            json!({
                "Text":nullable,"Word":word,"Small":small,"Wide":{"type":"integer"},
                "StringCase":{"type":"string"},"IntegerCase":{"type":"integer"},
                "Choice":{"oneOf":[branch,{"$ref":"#/components/schemas/IntegerCase"}]},
                "Ambiguous":{"oneOf":[{"type":"number"},{"type":"integer"}]},
                "Record":{"type":"object","required":["value","word","small","wide"],"additionalProperties":false,
                    "properties":{
                        "value":reference,"word":{"$ref":"#/components/schemas/Word"},
                        "small":{"$ref":"#/components/schemas/Small"},"wide":{"$ref":"#/components/schemas/Wide"},
                        "optional":{"$ref":"#/components/schemas/Word"},"maybe":{"$ref":"#/components/schemas/Text"},
                        "list":{"type":"array","items":{"$ref":"#/components/schemas/Text"}},
                        "choice":{"$ref":"#/components/schemas/Choice"},"ambiguous":{"$ref":"#/components/schemas/Ambiguous"},
                        "legacy":{"$ref":"legacy.json#/components/schemas/LegacyText"},
                        "legacyCount":{"$ref":"legacy.json#/components/schemas/LegacyCount"}
                    }}
            }),
        ),
        Some(
            json!({"openapi":"3.0.4","info":{"title":"Legacy","version":"1"},"paths":{},"components":{"schemas":{
                "LegacyText":{"type":"string","nullable":true},
                "LegacyCount":{"type":"integer","minimum":-1,"maximum":256,"exclusiveMinimum":true,"exclusiveMaximum":true}
            }}}),
        ),
    )
}

const GOOD: &str = r#"{"value":null,"word":"live","small":0,"wide":9007199254740993,"choice":"text","legacy":null,"legacyCount":255,"list":[null,"x"]}"#;
fn invalid_inputs() -> Vec<String> {
    let good: Value = serde_json::from_str(GOOD).unwrap();
    let mut cases = Vec::new();
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("value");
    cases.push(missing.to_string());
    for (name, value) in [
        ("value", json!(7)),
        ("word", Value::Null),
        ("small", json!(-1)),
        ("small", json!(256)),
        ("optional", Value::Null),
        ("ambiguous", json!(1)),
        ("legacyCount", json!(256)),
        ("legacy", json!(7)),
        ("wide", serde_json::from_str("1e-400").unwrap()),
    ] {
        let mut invalid = good.clone();
        invalid[name] = value;
        cases.push(invalid.to_string());
    }
    cases
}
fn run(command: &mut Command) {
    let description = format!("{command:?}");
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{description}: {error}"));
    assert!(
        output.status.success(),
        "{description}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("passed: {description}");
}

#[test]
fn selected_models_prune_ignored_children_and_preserve_presence_and_naming() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let root = root(&contract, "Record");
        let roots = [root.clone()];
        let rust = rust_models::plan_models(&contract, &roots);
        assert!(!rust.has_errors(), "{:?}", rust.diagnostics());
        let python = python_models::plan_models(&contract, &roots);
        assert!(!python.has_errors(), "{:?}", python.diagnostics());
        let go = go_models::plan_models(&contract, &roots);
        assert!(!go.has_errors(), "{:?}", go.diagnostics());
        let ts = typescript::plan_models(&contract, &roots, &[typescript::ModelView::Neutral]);
        assert!(!ts.has_errors(), "{:?}", ts.diagnostics());
        for symbol in rust.symbols() {
            assert!(!symbol.source().pointer().contains("Ignored"));
        }
        for symbol in python.symbols() {
            assert!(!symbol.source().pointer().contains("Ignored"));
        }
        for symbol in go.symbols() {
            assert!(!symbol.source().pointer().contains("Ignored"));
        }
        for symbol in ts.symbols() {
            assert!(!symbol.source().pointer().contains("Ignored"));
        }
        for (name, required, nullable) in [
            ("value", true, true),
            ("word", true, false),
            ("optional", false, false),
            ("maybe", false, true),
            ("legacy", false, true),
        ] {
            let field = python
                .fields()
                .iter()
                .find(|field| field.source == root.child("properties").child(name))
                .unwrap();
            assert_eq!((field.required, field.nullable), (required, nullable));
            let field = go
                .fields()
                .iter()
                .find(|field| field.source == root.child("properties").child(name))
                .unwrap();
            assert_eq!((field.required, field.nullable), (required, nullable));
        }
        assert_eq!(
            python
                .symbols()
                .iter()
                .find(|symbol| symbol.source() == &root)
                .unwrap()
                .name(),
            "Record"
        );
    }
}

#[test]
fn directional_requirements_stay_located_and_admitted_reference_intersections_remain_checked() {
    for schema in [
        json!({"type":"object","required":["id"],"properties":{"id":{"type":"string","readOnly":true}}}),
        json!({"allOf":[{"required":["id"]},{"properties":{"id":{"type":"string","writeOnly":true}}}]}),
    ] {
        let contract = load(envelope("3.0.4", json!({"Record":schema})), None);
        let roots = [root(&contract, "Record")];
        for errors in [
            rust_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
            python_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
            go_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
        ] {
            assert!(
                errors.iter().any(
                    |error| error.code == "oas30-directional-required-unsupported"
                        && error.source.pointer().ends_with("/required")
                        && error.at.end > error.at.start
                ),
                "{errors:?}"
            );
        }
        for view in [
            typescript::ModelView::Neutral,
            typescript::ModelView::Request,
            typescript::ModelView::Response,
        ] {
            let plan = typescript::plan_models(&contract, &roots, &[view]);
            assert!(
                plan.diagnostics()
                    .iter()
                    .any(|error| error.code == "oas30-directional-required-unsupported")
            );
        }
    }
    let contract = load(
        envelope(
            "3.1.2",
            json!({"Record":{"$ref":"#/components/schemas/Base","type":"string"},"Base":{"type":["string","null"]}}),
        ),
        None,
    );
    let roots = [root(&contract, "Record")];
    // Retained base-only representations can still decline this intersection;
    // the admitted native paths must preserve its non-null string assertion.
    assert!(rust_codecs::plan_codecs(contract.clone(), &roots, Default::default()).is_err());
    assert!(rust_codecs::plan_codecs_v2(contract.clone(), &roots, Default::default()).is_ok());
    let python = python_codecs::plan_codecs(contract.clone(), &roots, Default::default()).unwrap();
    let go = go_codecs::plan_codecs(contract.clone(), &roots, Default::default()).unwrap_err();
    assert!(
        go.iter()
            .any(|error| error.code == "ref-sibling-representation"
                && error.source == roots[0]
                && !error.at.is_empty())
    );
    let program = python.validation_program();
    program.check().unwrap();
    let root = program
        .roots
        .iter()
        .find(|root| root.source.pointer == roots[0].pointer())
        .unwrap();
    assert!(program.nodes[root.target].checks.iter().any(|check|
        matches!(&check.instruction, ProgramInstruction::Type { types } if types.as_slice() == [ProgramType::String])));
    let validator = OwnedCompiler::new(Config::default())
        .compile_v2(contract.clone(), &roots)
        .unwrap();
    assert!(matches!(
        validator.validate(&roots[0], &json!("text")),
        OwnedOutcome::Valid
    ));
    for value in [Value::Null, json!(42)] {
        assert!(matches!(
            validator.validate(&roots[0], &value),
            OwnedOutcome::Invalid(_)
        ));
    }
    // TypeScript has an actual intersection representation; null stays excluded.
    assert!(typescript::codecs::plan_codecs(contract, &roots, Default::default()).is_ok());
}

#[test]
#[ignore = "requires installed Cargo"]
fn rust_native_dialect_models_and_codecs() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let roots = [root(&contract, "Record")];
        let plan = rust_codecs::plan_codecs(contract, &roots, Default::default()).unwrap();
        let directory = artifact_dir("dialect-rust");
        suspect_codegen::write_files(&plan.render(), &directory).unwrap();
        std::fs::create_dir_all(directory.join("rust/tests")).unwrap();
        let invalid = invalid_inputs()
            .iter()
            .map(|text| format!("{text:?}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            r#"
use generated_models::{{models::*,codecs::*,Nullable,Presence}};
#[test] fn consumer() {{
    let small: u8 = SmallCodec::decode("255.0").unwrap(); assert_eq!(small,255);
    let mut record: Record = RecordCodec::decode({good:?}).unwrap();
    assert!(matches!(record.value,Nullable::Null));
    assert!(matches!(record.maybe,Presence::Absent)); assert!(record.optional.is_none());
    assert_eq!(record.wide.as_str(),"9007199254740993");
    record.maybe=Presence::Null;
    let wire=RecordCodec::encode(&record).unwrap();
    assert!(matches!(RecordCodec::decode(&wire).unwrap().maybe,Presence::Null));
    record.value=Nullable::Value("native".to_owned());
    assert!(RecordCodec::encode(&record).is_ok());
    record.value=Nullable::Value(String::new());
    assert!(matches!(RecordCodec::encode(&record),Err(CodecError::Invalid(_))));
    for input in [{invalid}] {{ assert!(matches!(RecordCodec::decode(input),Err(CodecError::Invalid(_))),"{{input}}"); }}
    assert!(ChoiceCodec::decode("1").is_ok());
}}
"#,
            good = GOOD
        );
        std::fs::write(directory.join("rust/tests/dialect.rs"), source).unwrap();
        run(Command::new("cargo")
            .args(["test", "--offline", "--quiet", "-j", "2", "--manifest-path"])
            .arg(directory.join("rust/Cargo.toml"))
            .arg("--target-dir")
            .arg(directory.join("build"))
            .env("RUSTFLAGS", "-D warnings"));
    }
}

#[test]
fn incomplete_nullability_proofs_are_located_refusals() {
    for schema in [
        json!({"$ref":"#/components/schemas/Record"}),
        json!({"$dynamicRef":"#unknown"}),
    ] {
        let contract = load(envelope("3.1.2", json!({"Record":schema})), None);
        let roots = [root(&contract, "Record")];
        for errors in [
            rust_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
            python_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
            go_models::plan_models(&contract, &roots)
                .diagnostics()
                .to_vec(),
        ] {
            assert!(
                errors
                    .iter()
                    .any(|error| error.kind == rust_models::DiagnosticKind::Error
                        && error.source.document() == roots[0].document()
                        && (error.source == roots[0]
                            || error
                                .source
                                .pointer()
                                .starts_with(&format!("{}/", roots[0].pointer())))
                        && error.at.end > error.at.start),
                "{errors:?}"
            );
        }
    }
}

#[test]
fn conditional_nullability_is_retained_by_the_admitted_scoped_codec_profile() {
    let contract = load(
        envelope(
            "3.1.2",
            json!({"Record":{"if":true,"then":{"type":"string"}}}),
        ),
        None,
    );
    let roots = [root(&contract, "Record")];
    assert!(rust_models::plan_models(&contract, &roots).has_errors());
    assert!(!rust_models::plan_models_v2(&contract, &roots).has_errors());
    for errors in [
        python_models::plan_models(&contract, &roots)
            .diagnostics()
            .to_vec(),
        go_models::plan_models(&contract, &roots)
            .diagnostics()
            .to_vec(),
    ] {
        assert!(
            !errors
                .iter()
                .any(|error| error.kind == rust_models::DiagnosticKind::Error),
            "{errors:?}"
        );
        assert!(
            errors.iter().any(
                |error| error.kind == rust_models::DiagnosticKind::CodecObligation
                    && error.source == roots[0]
                    && !error.at.is_empty()
            ),
            "{errors:?}"
        );
    }
    let python = python_codecs::plan_codecs(contract.clone(), &roots, Default::default()).unwrap();
    let go = go_codecs::plan_codecs(contract.clone(), &roots, Default::default()).unwrap();
    for program in [python.validation_program(), go.validation_program()] {
        assert_eq!(
            (program.version, program.profile),
            (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
        );
        assert!(
            program
                .nodes
                .iter()
                .flat_map(|node| &node.checks)
                .any(|check| matches!(check.instruction, ProgramInstruction::If { .. }))
        );
    }
    let validator = OwnedCompiler::new(Config::default())
        .compile_v2(contract, &roots)
        .unwrap();
    assert!(matches!(
        validator.validate(&roots[0], &json!("text")),
        OwnedOutcome::Valid
    ));
    assert!(matches!(
        validator.validate(&roots[0], &Value::Null),
        OwnedOutcome::Invalid(_)
    ));
}

#[test]
#[ignore = "requires installed Python"]
fn python_native_dialect_models_and_codecs() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let roots = [root(&contract, "Record")];
        let plan = python_codecs::plan_codecs(contract, &roots, Default::default()).unwrap();
        let directory = artifact_dir("dialect-python");
        suspect_codegen::write_files(&plan.render(), &directory).unwrap();
        let source = format!(
            r#"
import models as M
import model_codecs as C
from codec_runtime import CodecError
record: M.Record = C.RecordCodec.decode({good:?})
assert record.value is None and record.maybe is M.UNSET and record.optional is M.UNSET
assert type(record.small) is int and record.small==0
assert type(record.wide) is int and record.wide==9007199254740993
constructed=M.Record(value=None,word='live',small=0,wide=9007199254740993)
assert C.RecordCodec.decode(C.RecordCodec.encode(constructed)).wide==9007199254740993
record.maybe=None
assert C.RecordCodec.decode(C.RecordCodec.encode(record)).maybe is None
record.value='native'
assert C.RecordCodec.decode(C.RecordCodec.encode(record)).value=='native'
for text in {invalid}:
    try: C.RecordCodec.decode(text)
    except CodecError: pass
    else: raise AssertionError(text)
record.value=''
try: C.RecordCodec.encode(record)
except CodecError: pass
else: raise AssertionError('invalid mutation')
assert C.ChoiceCodec.decode('1')==1
"#,
            good = GOOD,
            invalid = serde_json::to_string(&invalid_inputs()).unwrap()
        );
        std::fs::write(directory.join("python/dialect_consumer.py"), source).unwrap();
        run(Command::new(
            std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()),
        )
        .arg("dialect_consumer.py")
        .current_dir(directory.join("python")));
    }
}

#[test]
#[ignore = "requires installed Go"]
fn go_native_dialect_models_and_codecs() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let roots = [root(&contract, "Record")];
        let plan = go_codecs::plan_codecs(contract, &roots, Default::default()).unwrap();
        let directory = artifact_dir("dialect-go");
        suspect_codegen::write_files(&plan.render(), &directory).unwrap();
        let invalid = invalid_inputs()
            .iter()
            .map(|text| format!("`{text}`"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            r#"package sdk
import "testing"
func TestDialectConsumer(t *testing.T) {{
    var record Record
    var err error
    record,err=Codecs.Record.Decode([]byte(`{GOOD}`));if err!=nil{{t.Fatal(err)}}
    if record.Value.IsValue || record.Maybe.IsSet || record.Optional.IsSet {{t.Fatal("presence and null changed")}}
    if record.Wide.String()!="9007199254740993"{{t.Fatal("lost exact integer")}}
    record.Maybe.IsSet=true;record.Maybe.Null=true
    output,err:=Codecs.Record.Encode(record);if err!=nil{{t.Fatal(err)}}
    repeated,err:=Codecs.Record.Decode(output);if err!=nil{{t.Fatal(err)}}
    if !repeated.Maybe.IsSet || !repeated.Maybe.Null {{t.Fatal("explicit null lost")}}
    for _,input:=range []string{{{invalid}}} {{ if _,err=Codecs.Record.Decode([]byte(input));err==nil{{t.Fatalf("accepted %s",input)}} }}
    if _,err=Codecs.Choice.Decode([]byte(`1`));err!=nil{{t.Fatal(err)}}
    record.Value.IsValue=true;record.Value.Value=""
    if _,err=Codecs.Record.Encode(record);err==nil{{t.Fatal("invalid mutation accepted")}}
}}
"#
        );
        std::fs::write(directory.join("go/dialect_test.go"), source).unwrap();
        run(Command::new("go")
            .args(["test", "-p", "2", "./..."])
            .env("GOTOOLCHAIN", "local")
            .current_dir(directory.join("go")));
    }
}

#[test]
#[ignore = "requires installed Node and TypeScript"]
fn typescript_native_dialect_models_and_codecs() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let roots = [root(&contract, "Record")];
        let plan = typescript::codecs::plan_codecs(contract, &roots, Default::default()).unwrap();
        let name = plan
            .models()
            .symbols()
            .iter()
            .find(|symbol| symbol.source() == &roots[0])
            .unwrap()
            .name()
            .to_owned();
        let directory = artifact_dir("dialect-ts");
        suspect_codegen::write_files(&plan.render(), &directory).unwrap();
        let source = format!(
            r#"
import {{ {name}Codec, ChoiceCodec, ModelCodecError }} from './model-codecs.js';
import type {{ {name} }} from './models.js';
const check=(ok:boolean)=>{{if(!ok)throw new Error('dialect consumer assertion');}};
const record: {name}={name}Codec.decode({good:?});
check(record.value===null && !Object.hasOwn(record,'maybe') && !Object.hasOwn(record,'optional'));
const nativeSmall:number=record.small; check(nativeSmall===0);
const exact:bigint=record.wide; check(exact===9007199254740993n);
record.maybe=null;
check({name}Codec.decode({name}Codec.encode(record)).maybe===null);
record.value='native'; check({name}Codec.decode({name}Codec.encode(record)).value==='native');
for(const input of {invalid}){{let rejected=false;try{{{name}Codec.decode(input)}}catch(error){{if(!(error instanceof ModelCodecError))throw error;rejected=true;}}check(rejected);}}
record.value='';let rejected=false;try{{{name}Codec.encode(record)}}catch(error){{if(!(error instanceof ModelCodecError))throw error;rejected=true;}}check(rejected);
check(ChoiceCodec.decode('1')===1n);
// @ts-expect-error nullable does not bypass the Word enum intersection
record.word=null;
// @ts-expect-error required nullable value still requires presence
const absent: {name}={{word:'live',small:0,wide:1n}};
"#,
            good = GOOD,
            invalid = serde_json::to_string(&invalid_inputs()).unwrap()
        );
        std::fs::write(directory.join("typescript/dialect-consumer.ts"), source).unwrap();
        run(Command::new("tsc")
            .args([
                "--target",
                "ES2022",
                "--module",
                "NodeNext",
                "--moduleResolution",
                "NodeNext",
                "--strict",
                "--exactOptionalPropertyTypes",
                "--noUncheckedIndexedAccess",
                "--outDir",
                "compiled",
                "dialect-consumer.ts",
            ])
            .current_dir(directory.join("typescript")));
        run(Command::new("node")
            .arg("compiled/dialect-consumer.js")
            .current_dir(directory.join("typescript")));
    }
    // Modern schema refs keep assertions; this representable intersection must
    // work as a native codec, not merely produce an expression string.
    let contract = load(
        envelope(
            "3.1.2",
            json!({
                "Record":{"$ref":"#/components/schemas/Base","type":"string"},
                "Base":{"type":["string","null"]}
            }),
        ),
        None,
    );
    let roots = [root(&contract, "Record")];
    let plan = typescript::codecs::plan_codecs(contract, &roots, Default::default()).unwrap();
    let directory = artifact_dir("dialect-ts-modern-ref");
    suspect_codegen::write_files(&plan.render(), &directory).unwrap();
    std::fs::write(directory.join("typescript/dialect-consumer.ts"), r#"
import { RecordCodec, ModelCodecError } from './model-codecs.js';
import type { Record } from './models.js';
const value:Record=RecordCodec.decode('"text"');
if(value!=='text')throw new Error('modern ref intersection');
let rejected=false;try{RecordCodec.decode('null')}catch(error){if(!(error instanceof ModelCodecError))throw error;rejected=true;}
if(!rejected)throw new Error('modern sibling type was ignored');
// @ts-expect-error modern ref siblings remove null from the native intersection
const invalid:Record=null;
"#).unwrap();
    run(Command::new("tsc")
        .args([
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--outDir",
            "compiled",
            "dialect-consumer.ts",
        ])
        .current_dir(directory.join("typescript")));
    run(Command::new("node")
        .arg("compiled/dialect-consumer.js")
        .current_dir(directory.join("typescript")));
}

#[test]
#[ignore = "requires installed Swift 6"]
fn swift_native_dialect_models_and_codecs() {
    for version in ["3.0.4", "3.1.2"] {
        let contract = fixture(version);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let plan = swift_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
        plan.program().check().unwrap();
        assert!(
            plan.program()
                .nodes
                .iter()
                .all(|node| !node.source.pointer.contains("Ignored"))
        );
        let directory = artifact_dir("dialect-swift");
        suspect_codegen::write_files(&plan.render(), &directory).unwrap();
        let target = directory.join("Tests/GeneratedSDKTests");
        std::fs::create_dir_all(&target).unwrap();
        let invalid = invalid_inputs()
            .iter()
            .map(|text| format!("#\"{text}\"#"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            r##"
import XCTest
import Foundation
import GeneratedSDK
final class DialectTests: XCTestCase {{
  func testConsumer() throws {{
    var record: Record = try Codecs.record.decode(Data(#"{GOOD}"#.utf8))
    if case .null = record.value {{}} else {{XCTFail("nullable required")}}
    if case .missing = record.maybe {{}} else {{XCTFail("absence")}}
    XCTAssertEqual(record.wide.raw,"9007199254740993")
    record.maybe = .null
    let again = try Codecs.record.decode(Codecs.record.encode(record))
    if case .null = again.maybe {{}} else {{XCTFail("explicit null")}}
    for input in [{invalid}] {{XCTAssertThrowsError(try Codecs.record.decode(Data(input.utf8)))}}
    record.value = .value("")
    XCTAssertThrowsError(try Codecs.record.encode(record))
    _ = try Codecs.choice.decode(Data("1".utf8))
  }}
}}
"##
        );
        std::fs::write(target.join("DialectTests.swift"), source).unwrap();
        run(Command::new("swift")
            .args(["test", "-j", "2", "--disable-swift-testing"])
            .current_dir(&directory));
    }
}
