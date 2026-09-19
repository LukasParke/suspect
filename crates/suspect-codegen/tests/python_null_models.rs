//! Null-only annotations must satisfy the native type checker and retain the
//! runtime NoneType aliases, omission states, and checked mutable codecs.
use serde_json::json;
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::python_codecs::plan_codecs;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn checked(command: &mut Command, root: &Path, label: &str, success: bool) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    std::fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    std::fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert_eq!(
        output.status.success(),
        success,
        "{command:?}\n{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !success {
        let diagnostics = String::from_utf8_lossy(&output.stdout);
        assert!(
            diagnostics.contains("[arg-type]"),
            "negative control must fail because a non-null value violates its native type"
        );
        for line in 2..=4 {
            assert!(
                diagnostics.contains(&format!("negative.py:{line}: error:")),
                "every forbidden constructor value needs its own type error: {diagnostics}"
            );
        }
    }
}

#[test]
#[ignore = "requires native Python 3.11+/mypy; executes generated models/codecs and controlled consumer type checks"]
fn null_only_fields_aliases_containers_and_unions_preserve_native_typing_and_values() {
    let parent = std::env::var_os("SUSPECT_TEST_ARTIFACT_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = tempfile::Builder::new()
        .prefix("python-null-models-")
        .tempdir_in(parent)
        .unwrap()
        .keep();
    println!("null-only model artifact: {}", root.display());
    let input = root.join("api.json");
    std::fs::write(&input,json!({"openapi":"3.1.2","info":{"title":"Null annotations","version":"1"},"paths":{},
        "components":{"schemas":{
            "Null":{"type":"null"},
            "Alias":{"$ref":"#/components/schemas/Null"},
            "Nulls":{"type":"array","items":{"type":"null"}},
            "Extras":{"type":"object","additionalProperties":{"type":"null"}},
            "SameNull":{"anyOf":[{"type":"null"},{"type":"null"}]},
            "ExclusiveNull":{"oneOf":[{"type":"null"},{"type":"null"}]},
            "NullFields":{"type":"object","additionalProperties":false,"required":["required"],"properties":{
                "required":{"$ref":"#/components/schemas/Alias"},"flag":{"type":"null"},
                "items":{"type":"array","items":{"type":"null"}},
                "extras":{"$ref":"#/components/schemas/Extras"},
                "choice":{"anyOf":[{"type":"null"},{"type":"string"}]},
                "same":{"anyOf":[{"type":"null"},{"type":"null"}]}
            }}
        }}}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap());
    let plan = plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        Default::default(),
    )
    .unwrap();
    suspect_codegen::write_files(&plan.render(), &root).unwrap();
    std::fs::write(
        root.join("consumer.py"),
        r#"from __future__ import annotations
import typing
from python import models as m, model_codecs as c
from python.codec_runtime import CodecError

def accepts_null(value: m.Alias) -> None:
    assert value is None

extra = m.Extras()
extra.set_extra('item', None)
null: m.Null = None
accepts_null(null)
value = m.NullFields(required=null, flag=None, items=[None], extras=extra, choice=None, same=None)
runtime_aliases: tuple[object, ...] = (m.Null, m.Alias)
assert all(alias is type(None) for alias in runtime_aliases)
assert type(None) in typing.get_args(typing.get_type_hints(m.NullFields)['flag'])
assert type(None) in typing.get_args(typing.get_type_hints(m.NullFields)['same'])
assert c.NullCodec.decode('null') is None
assert c.NullsCodec.decode('[null,null]') == [None,None]
assert c.SameNullCodec.decode('null') is None
wire = c.NullFieldsCodec.encode(value)
assert c.NullFieldsCodec.decode(wire).flag is None
omitted = m.NullFields(required=None)
assert omitted.flag is m.UNSET
assert c.NullFieldsCodec.decode(c.NullFieldsCodec.encode(omitted)).flag is m.UNSET
omitted_wire = c.NullFieldsCodec.encode_value(omitted)
present_wire = c.NullFieldsCodec.encode_value(value)
assert isinstance(omitted_wire, dict) and isinstance(present_wire, dict)
assert 'flag' not in omitted_wire and present_wire['flag'] is None
try:
    c.ExclusiveNullCodec.decode('null')
except CodecError as error:
    assert error.kind == 'invalid'
else:
    raise AssertionError('exactly-one null overlap was accepted')
setattr(value, 'flag', 'not null')
try:
    c.NullFieldsCodec.encode(value)
except CodecError as error:
    assert error.kind == 'conversion' and error.path == '/flag'
else:
    raise AssertionError('a mutated non-null field passed the checked codec')
print('NULL_ONLY_MODELS_CHECKED')
"#,
    )
    .unwrap();
    std::fs::write(root.join("negative.py"),"from python import models as m\nm.NullFields(required=1)\nm.NullFields(required=None, flag='not null')\nm.NullFields(required=None, items=[False])\n").unwrap();
    let python = std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into());
    let tools = std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    let mut mypy = Command::new(&tools);
    mypy.current_dir(&root)
        .args([
            "-m",
            "mypy",
            "--strict",
            "--python-version",
            "3.11",
            "--python-executable",
        ])
        .arg(&python)
        .args(["--cache-dir", "mypy-cache", "python", "consumer.py"]);
    checked(&mut mypy, &root, "mypy", true);
    checked(
        Command::new(&python).current_dir(&root).arg("consumer.py"),
        &root,
        "runtime",
        true,
    );
    checked(
        Command::new(&tools)
            .current_dir(&root)
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--python-executable",
            ])
            .arg(&python)
            .args(["--cache-dir", "negative-cache", "negative.py"]),
        &root,
        "negative-types",
        false,
    );
}
