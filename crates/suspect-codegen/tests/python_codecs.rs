//! Public model/codec boundary, exercised in a native Python interpreter.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::python_codecs::{CodecConfig, CodecPlan, plan_codecs};
use suspect_ir::contract::Contract;
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
fn fixture() -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Codec","version":"1"},"paths":{},"components":{"schemas":{
        "Account":{"type":"object","required":["key","big","precise","nullable"],"properties":{"key":{"type":"string","minLength":1},"big":{"type":"integer"},"precise":{"type":"number"},"nullable":{"type":["string","null"]},"note":{"type":"string"},"child":{"$ref":"#/components/schemas/Account"},"payload":{"oneOf":[{"$ref":"#/components/schemas/Text"},{"$ref":"#/components/schemas/Flag"}]}}},
        "Text":{"type":"object","required":["kind","text"],"properties":{"kind":{"type":"string","const":"text"},"text":{"type":"string"}}},
        "Flag":{"type":"object","required":["kind","enabled"],"properties":{"kind":{"type":"string","const":"flag"},"enabled":{"type":"boolean"}}},
        "Lookup":{"type":"object","additionalProperties":{"type":"integer"}},
        "Ambiguous":{"oneOf":[{"type":"integer"},{"type":"number"}]},
        "Colour":{"type":"string","enum":["red","green"]}
    }}}).to_string()).unwrap();
    load(&path)
}
fn planned() -> CodecPlan {
    let c = fixture();
    plan_codecs(c.clone(), c.schema_roots(), Default::default()).unwrap()
}
fn native(plan: &CodecPlan, consumer: &str) {
    let root = tempfile::tempdir().unwrap().keep();
    suspect_codegen::write_files(&plan.render(), &root).unwrap();
    let python = root.join("python");
    std::fs::write(python.join("consumer.py"), consumer).unwrap();
    let output =
        Command::new(std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()))
            .arg("consumer.py")
            .current_dir(&python)
            .output()
            .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn plans_preserve_source_identities_and_validate_every_public_root() {
    let plan = planned();
    let files = plan.render();
    for path in [
        "python/models.py",
        "python/model_codecs.py",
        "python/codec_runtime.py",
        "python/json_runtime.py",
        "python/validation.py",
        "python/validation_number.py",
        "python/validation_program.json",
        "python/codec-plan.json",
    ] {
        assert!(files.iter().any(|file| file.path == path), "{path}");
    }
    let program: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "python/validation_program.json")
            .unwrap()
            .content,
    )
    .unwrap();
    for symbol in plan.models().symbols() {
        assert!(
            program["roots"]
                .as_array()
                .unwrap()
                .iter()
                .any(|root| root["source"]["pointer"] == symbol.source().pointer())
        );
    }
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "python/manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["source_codecs"], true);
    assert_eq!(manifest["model_only"], false);
}

#[test]
#[ignore = "requires native Python"]
fn exact_presence_union_recursive_and_mutated_models_are_checked() {
    native(
        &planned(),
        r#"
import model_codecs as C
import models as M
from codec_runtime import CodecError
from json_runtime import JsonNumber, parse_json
text='{"key":"a","big":9007199254740993,"precise":1e-400,"nullable":null,"payload":{"kind":"text","text":"hi"},"extra":7}'
account=C.AccountCodec.decode(text)
assert account.big==9007199254740993 and account.precise.token=='1e-400'
assert account.nullable is None and account.note is M.UNSET and account.child is M.UNSET
assert isinstance(account.payload,M.Text) and account.payload.kind=='text'
assert account.extra_fields['extra'].token=='7'
wire=C.AccountCodec.encode_value(account)
assert 'note' not in wire and 'child' not in wire and wire['nullable'] is None
assert wire['big'].token=='9007199254740993'
account.child=C.AccountCodec.decode(text)
assert C.AccountCodec.decode(C.AccountCodec.encode(account)).child.key=='a'
account.key=''
try: C.AccountCodec.encode(account)
except CodecError as error: assert error.kind=='invalid'
else: raise AssertionError('mutated invalid key accepted')
assert M.Text(text='tag').kind=='text'
for invalid in ['{"key":"a","big":1,"precise":2}', '{"key":"a","big":true,"precise":2,"nullable":null}']:
    try: C.AccountCodec.decode(invalid)
    except CodecError: pass
    else: raise AssertionError('invalid requiredness/type accepted')
try: C.AmbiguousCodec.decode('1')
except CodecError as error: assert error.kind=='invalid'
else: raise AssertionError('oneOf ambiguity accepted')
assert C.AmbiguousCodec.decode('1.5').token=='1.5'
lookup=C.LookupCodec.decode('{"one":9007199254740993}')
assert lookup.extra_fields['one']==9007199254740993
lookup.set_extra('two',2)
assert C.LookupCodec.decode(C.LookupCodec.encode(lookup)).extra_fields['two']==2
try: C.ColourCodec.encode('blue')
except CodecError: pass
else: raise AssertionError('enum mutation accepted')
"#,
    );
}

#[test]
#[ignore = "requires native Python"]
fn incomplete_validation_and_cycles_are_not_accepted() {
    let c = fixture();
    let mut config = CodecConfig::default();
    config.schema.max_evaluation_steps = 1;
    let plan = plan_codecs(c.clone(), c.schema_roots(), config).unwrap();
    native(
        &plan,
        r#"from model_codecs import ColourCodec
from codec_runtime import CodecError
try: ColourCodec.decode('"red"')
except CodecError as error: assert error.kind=='resource'
else: raise AssertionError('incomplete validation accepted')
"#,
    );
    native(
        &planned(),
        r#"from model_codecs import AccountCodec
from codec_runtime import CodecError
a=AccountCodec.decode('{"key":"a","big":1,"precise":2,"nullable":null}')
a.child=a
try: AccountCodec.encode(a)
except CodecError as error: assert error.kind in ('conversion','resource')
else: raise AssertionError('native cycle accepted')
"#,
    );
}

#[test]
#[ignore = "requires native Python and tracked OpenRouter public source"]
fn tracked_credits_codec_preserves_real_exact_fields() {
    let source = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    )
    .join("projects/docs/openapi/openapi.yaml");
    let c = load(&source);
    let op = c
        .operations()
        .find(|op| op.operation_id() == Some("getCredits"))
        .unwrap();
    let schema = op
        .responses()
        .into_iter()
        .find(|r| r.status_key() == "200")
        .unwrap()
        .content()[0]
        .schema()
        .unwrap()
        .id()
        .clone();
    let plan = plan_codecs(c, std::slice::from_ref(&schema), Default::default()).unwrap();
    let name = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.source() == &schema)
        .unwrap()
        .name();
    native(
        &plan,
        &format!(
            "from model_codecs import {name}Codec as codec\na=codec.decode('{{\"data\":{{\"total_credits\":9007199254740993.25,\"total_usage\":1e-400}}}}')\nassert a.data.total_credits.token=='9007199254740993.25'\nassert a.data.total_usage.token=='1e-400'\nassert codec.decode(codec.encode(a)).data.total_usage.token=='1e-400'\n"
        ),
    );
}
