//! Canonical source -> native Python model package and explicit codec obligations.

use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::python_models::{ModelPlan, plan_models};
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

fn fixture(schemas: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, json!({"openapi":"3.1.0","info":{"title":"Python models","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
    load(&path)
}

fn native(plan: &ModelPlan, source: &str) {
    assert!(!plan.release_ready());
    let directory = tempfile::tempdir().unwrap().keep();
    suspect_codegen::write_files(&plan.render().unwrap(), &directory).unwrap();
    std::fs::write(directory.join("consumer.py"), source).unwrap();
    let output =
        Command::new(std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()))
            .current_dir(&directory)
            .arg("consumer.py")
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

#[test]
fn unsupported_python_shapes_fail_before_artifacts() {
    for schema in [
        json!({"$dynamicRef":"#node"}),
        json!({"type":"object","required":42}),
        json!({"type":["string",42]}),
    ] {
        let contract = fixture(json!({"Rejected":schema}));
        let plan = plan_models(&contract, contract.schema_roots());
        assert!(plan.has_errors(), "{:?}", plan.diagnostics());
        assert!(plan.render().is_err());
        assert!(plan.diagnostics().iter().any(|error| {
            error
                .source
                .pointer()
                .starts_with("/components/schemas/Rejected")
                && !error.at.is_empty()
        }));
    }
}

#[test]
fn newly_represented_python_shapes_keep_model_only_obligations() {
    for schema in [
        json!({"type":"object","additionalProperties":{"type":"integer"}}),
        json!({"type":["object","null"]}),
        json!({"oneOf":[{"type":"string"},{"type":"integer"}]}),
        json!({"type":["object","null"],"properties":{"named":{"type":"string"}}}),
        json!({"allOf":[{"type":"string"},{"type":"integer"}]}),
        json!({"type":"object","patternProperties":{"^x":{"type":"number"}},"additionalProperties":false}),
    ] {
        let contract = fixture(json!({"Model":schema}));
        let plan = plan_models(&contract, contract.schema_roots());
        assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
        assert!(plan.render().is_ok());
        assert!(!plan.release_ready());
        assert!(
            plan.diagnostics()
                .iter()
                .any(|finding| finding.code == "python-model-codec-unimplemented")
        );
    }
}

#[test]
#[ignore = "requires native Python 3.11 or newer"]
fn native_dataclasses_keep_presence_refs_names_and_inert_documentation() {
    let contract = fixture(json!({
        "A":{"$ref":"#/components/schemas/Z"},"Z":{"type":["string","null"]},"Null":{"type":"null"},
        "Node":{"type":"object","required":["label","required_maybe"],"description":"\"\"\"\nraise RuntimeError('source prose')\n<script>not active</script>","properties":{
            "label":{"type":"string"},"required_maybe":{"type":["string","null"]},
            "optional":{"type":"string"},"maybe":{"type":["string","null"]},"next":{"$ref":"#/components/schemas/Node"},
            "alias":{"$ref":"#/components/schemas/A"},"class":{"type":"string"}
        }},
        "Unset":{"type":"string"},"Name\nraise RuntimeError('source pointer')":{"type":"integer"}
    }));
    let plan = plan_models(&contract, contract.schema_roots());
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    native(
        &plan,
        r#"
import html as html_module, inspect, pydoc, typing
import python as package
from python import models
from python.json_runtime import JsonNumber, JsonValue as WireValue
node=models.Node(label='root',required_maybe=None)
assert node.optional is models.UNSET and node.maybe is models.UNSET
node.maybe=None
node.next=models.Node(label='child',required_maybe='present')
node.alias=None
assert node.maybe is None and node.next.label=='child'
assert models.Null is type(None)
assert typing.get_args(models.A)==typing.get_args(models.Z)
assert type(None) in typing.get_args(typing.get_type_hints(models.Node)['alias'])
JsonValue = int  # A caller's homonym must not capture recursive runtime aliases.
def annotated(value: WireValue) -> WireValue: return value
resolved=typing.get_type_hints(annotated)['value']
list_type=next(arg for arg in typing.get_args(resolved) if typing.get_origin(arg) is list)
assert str in typing.get_args(typing.get_args(list_type)[0])
assert package.Node is models.Node
try: models.Node(label='missing')
except TypeError: pass
else: raise AssertionError('required nullable value became optional')
try: models.Node('positional',None)
except TypeError: pass
else: raise AssertionError('constructor is not keyword-only')
node.set_extra('decimal',JsonNumber('9007199254740993.01'))
assert node.extra_fields['decimal'].token=='9007199254740993.01'
try: node.set_extra('label','shadow')
except ValueError: pass
else: raise AssertionError('extra shadows declared wire key')
try: node.extra_fields['label']='shadow'
except TypeError: pass
else: raise AssertionError('extra storage is directly mutable')
html=pydoc.HTMLDoc().docmodule(models)
rendered=html_module.unescape(html).replace('\xa0',' ')
assert 'Node' in rendered and 'source prose' in rendered
assert '<script>not active</script>' not in html
assert inspect.signature(models.Node).parameters['optional'].default is models.UNSET
"#,
    );
}

#[test]
#[ignore = "requires native Python and the tracked OpenRouter checkout"]
fn tracked_credits_closure_has_native_exact_number_fields() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let response = contract
        .operations()
        .find(|op| op.operation_id() == Some("getCredits"))
        .unwrap()
        .responses()
        .into_iter()
        .find(|response| {
            response.status() == Some(suspect_ir::contract::ResponseStatus::Exact(200))
        })
        .unwrap()
        .content()
        .into_iter()
        .find(|media| media.name() == "application/json")
        .unwrap()
        .schema()
        .unwrap()
        .id()
        .clone();
    let plan = plan_models(&contract, &[response]);
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    let data = plan
        .fields()
        .iter()
        .find(|field| field.wire == "total_credits")
        .unwrap()
        .model
        .clone();
    native(
        &plan,
        &format!(
            r#"
from python import models
from python.json_runtime import JsonNumber
data=models.{data}(total_credits=JsonNumber('9007199254740993.0001'),total_usage=JsonNumber('1e-400'))
assert data.total_credits.token=='9007199254740993.0001'
assert data.total_usage.token=='1e-400'
"#
        ),
    );
}
