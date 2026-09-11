//! Current source Contract -> compile_v2 -> actual native Python validation.
//! The maintained 32-case source fixture is independent of this adapter.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{python_json, python_validation};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn artifacts(label: &str) -> PathBuf {
    let parent = std::env::var_os("SUSPECT_PYTHON_SCOPED_ARTIFACTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("sdk-python-scoped-{label}-"))
        .tempdir_in(parent)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

fn compile(
    root: &Path,
    name: &str,
    schema: Value,
    config: Config,
) -> (Arc<Contract>, SourceId, OwnedProgram) {
    let directory = root.join(name);
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("api.json");
    fs::write(&path, json!({"openapi":"3.1.2","info":{"title":"Scoped Python witness","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&directory).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let source = SourceId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    let program = OwnedCompiler::new(config)
        .compile_v2(contract.clone(), std::slice::from_ref(&source))
        .unwrap()
        .program();
    (contract, source, program)
}

fn limits(case: &Value) -> Config {
    let mut config = Config::default();
    for (name, value) in case["limits"].as_object().into_iter().flatten() {
        let value = value.as_u64().unwrap() as usize;
        match name.as_str() {
            "maxDepth" => config.max_depth = value,
            "maxErrors" => config.max_errors = value,
            "maxNumberBytes" => config.max_number_bytes = value,
            "maxEqualitySteps" => config.max_equality_steps = value,
            "maxEvaluationSteps" => config.max_evaluation_steps = value,
            _ => panic!("unknown fixture limit"),
        }
    }
    config
}

fn instance_path(id: &str) -> &'static str {
    match id {
        "contains-zero-does-not-mark-unmatched" | "contains-exact-integrality" => "/0",
        "contains-failure-after-exceeded-maximum" => "/1",
        "pattern-overlap-rejects" | "named-and-pattern-both-apply" => "/x",
        "property-names-checks-key-not-value" => "/long",
        "property-names-does-not-annotate-values" => "/ok",
        "failed-anyof-branch-does-not-leak"
        | "allof-cousins-have-independent-scopes"
        | "not-discards-annotations"
        | "required-is-not-an-evaluation" => "/a",
        "nested-members-do-not-mark-parent" => "/inner",
        "prefix-and-contains-leave-unmatched-item" => "/2",
        _ => "",
    }
}

fn vectors(root: &Path) -> (OwnedProgram, Value) {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    let mut records = Vec::new();
    let mut first = None;
    for case in fixture["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let schema = serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap();
        let (_, _, program) = compile(root, id, schema, limits(case));
        if first.is_none() {
            first = Some(program.clone());
        }
        assert_eq!(program.version, OwnedProgram::V2_VERSION, "{id}");
        records.push(json!({"id":id,"program":program,"root":program.roots[0].target,"instance":case["instanceJson"],"expected":case["expected"],"source":case["source"],"instancePath":instance_path(id)}));
    }
    assert_eq!(records.len(), 32);
    // Independently counted from the documented algorithm: each branch costs
    // seven visits; duplicate candidate insertion costs remain observable.
    let duplicate = json!({"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false});
    for (budget, expected, at) in [
        (18, "EvaluationFailure", "/components/schemas/Root/allOf"),
        (
            20,
            "EvaluationFailure",
            "/components/schemas/Root/unevaluatedProperties",
        ),
        (21, "Valid", ""),
    ] {
        let config = Config {
            max_evaluation_steps: budget,
            ..Default::default()
        };
        let id = format!("duplicate-merge-budget-{budget}");
        let (_, _, program) = compile(root, &id, duplicate.clone(), config);
        records.push(json!({"id":id,"program":program,"root":program.roots[0].target,"instance":"{\"a\":1}","expected":expected,"source":if at.is_empty(){Value::Null}else{json!(at)},"instancePath":""}));
    }
    // Distinct decoded keys and a member name requiring JSON-Pointer escaping.
    for (id, schema, instance, expected, at, path) in [
        (
            "unicode-key-identity",
            json!({"properties":{"é":true},"unevaluatedProperties":false}),
            "{\"é\":1}",
            "Invalid",
            "/components/schemas/Root/unevaluatedProperties",
            "/é",
        ),
        (
            "property-name-escaped-path",
            json!({"propertyNames":{"maxLength":1},"unevaluatedProperties":true}),
            "{\"a/b~\":1}",
            "Invalid",
            "/components/schemas/Root/propertyNames/maxLength",
            "/a~1b~0",
        ),
        (
            "allof-failure-clears-annotations",
            json!({"allOf":[{"properties":{"a":true}},false],"unevaluatedProperties":{"type":"integer"}}),
            "{\"a\":12345}",
            "EvaluationFailure",
            "/components/schemas/Root/unevaluatedProperties/type",
            "/a",
        ),
        (
            "oneof-overlap-does-not-merge",
            json!({"oneOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":{"type":"integer"}}),
            "{\"a\":12345}",
            "EvaluationFailure",
            "/components/schemas/Root/unevaluatedProperties/type",
            "/a",
        ),
        (
            "if-passing-condition-survives-failed-then-locally",
            json!({"if":{"properties":{"a":true}},"then":false,"unevaluatedProperties":{"type":"integer"}}),
            "{\"a\":12345}",
            "Invalid",
            "/components/schemas/Root/then",
            "",
        ),
        (
            "contains-count-failure-retains-local-matches",
            json!({"contains":true,"maxContains":0,"unevaluatedItems":{"type":"integer"}}),
            "[12345]",
            "Invalid",
            "/components/schemas/Root/maxContains",
            "",
        ),
        (
            "scoped-enum-skips-unused-exact-literal",
            json!({"if":{"enum":[1,12345]},"then":true}),
            "1",
            "Valid",
            "",
            "",
        ),
        (
            "recursive-same-instance",
            json!({"$ref":"#/components/schemas/Root","if":true}),
            "{}",
            "EvaluationFailure",
            "/components/schemas/Root",
            "",
        ),
        (
            "recursive-property-name-instance",
            json!({"propertyNames":{"$ref":"#/components/schemas/Root/$defs/Name"},"$defs":{"Name":{"$ref":"#/components/schemas/Root/$defs/Name"}}}),
            "{\"name\":1}",
            "EvaluationFailure",
            "/components/schemas/Root/$defs/Name",
            "/name",
        ),
        (
            "productive-deep-object",
            json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}),
            "{}",
            "Valid",
            "",
            "",
        ),
    ] {
        let (_, _, program) = compile(
            root,
            id,
            schema,
            Config {
                max_number_bytes: 3,
                ..Default::default()
            },
        );
        records.push(json!({"id":id,"program":program,"root":program.roots[0].target,"instance":instance,"expected":expected,"source":at,"instancePath":path}));
    }
    (
        first.unwrap(),
        json!({"format":"python-scoped-source-vectors-v1","cases":records}),
    )
}

fn checked(command: &mut Command, root: &Path, label: &str) {
    let output = command
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn v2_emission_is_additive_and_v1_program_bytes_are_identical() {
    let root = artifacts("admission");
    let config = Config::default();
    let (contract, source, modern) = compile(
        &root,
        "base",
        json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
        config.clone(),
    );
    let legacy = OwnedCompiler::new(config)
        .compile(contract, &[source])
        .unwrap()
        .program();
    assert_eq!(modern.version, OwnedProgram::V1_VERSION);
    assert_eq!(
        serde_json::to_vec(&modern).unwrap(),
        serde_json::to_vec(&legacy).unwrap()
    );
    let legacy_files = python_validation::emit(&legacy).unwrap();
    assert_eq!(legacy_files, python_validation::emit(&modern).unwrap());
    assert_eq!(legacy_files.len(), 3);
    let (_, _, modern) = compile(
        &root,
        "scoped",
        json!({"contains":{"type":"integer"},"unevaluatedItems":false}),
        Default::default(),
    );
    assert_eq!(modern.version, OwnedProgram::V2_VERSION);
    let files = python_validation::emit(&modern).unwrap();
    assert!(
        files
            .iter()
            .any(|file| file.path == "python/validation_v1.py")
    );
    let mut forged = modern;
    forged.version = OwnedProgram::V1_VERSION;
    forged.profile = OwnedProgram::V1_PROFILE;
    assert!(
        python_validation::emit(&forged).is_err(),
        "v1 must refuse scoped instructions before emission"
    );
    forged.version = OwnedProgram::V3_VERSION;
    forged.profile = OwnedProgram::V3_PROFILE;
    assert!(
        python_validation::emit(&forged).is_err(),
        "v3 requires indexed resource metadata"
    );
}

#[test]
#[ignore = "requires Python 3.11 and 3.14; compiles maintained source fixtures through compile_v2"]
fn scoped_python_runtime_matches_source_vectors_guards_and_exact_budgets() {
    let root = artifacts("runtime");
    let (program, vectors) = vectors(&root);
    let mut files = python_validation::emit(&program).unwrap();
    files.extend(python_json::emit());
    files.push(suspect_codegen::OutFile {
        path: "python/__init__.py".into(),
        content: String::new(),
    });
    suspect_codegen::write_files(&files, &root).unwrap();
    fs::write(
        root.join("python/vectors.json"),
        serde_json::to_string(&vectors).unwrap(),
    )
    .unwrap();
    fs::write(root.join("python/consumer.py"), RUNTIME).unwrap();
    for version in ["3.11", "3.14"] {
        checked(
            Command::new(format!("python{version}"))
                .arg("consumer.py")
                .current_dir(root.join("python")),
            &root,
            &format!("runtime-{version}"),
        );
    }
    let tools = std::env::var_os("SUSPECT_PYTHON_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/sdk-native-python-tools/bin/python")
        });
    checked(
        Command::new(tools)
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--cache-dir",
            ])
            .arg(root.join("mypy-cache"))
            .arg(root.join("python")),
        &root,
        "mypy-runtime",
    );
    println!("Python v2 runtime evidence: {}", root.display());
}

const RUNTIME: &str = r#"from __future__ import annotations
import copy
import json
import sys
from pathlib import Path
from typing import Any, Callable, TYPE_CHECKING
if TYPE_CHECKING or __package__:
    from .json_runtime import JsonNumber, parse_json
    from .validation import ValidationSession, ValidationError, ProgramError
else:
    from json_runtime import JsonNumber, parse_json
    from validation import ValidationSession, ValidationError, ProgramError

data = json.loads(Path('vectors.json').read_text(), parse_int=JsonNumber, parse_float=JsonNumber)
cases = data['cases']
for case in cases:
    session = ValidationSession(case['program'])
    root = case['root'].to_int()
    try:
        session.check(root, parse_json(case['instance']))
    except ValidationError as error:
        expected = 'EvaluationFailure' if error.kind == 'evaluation_failure' else 'Invalid'
        assert case['expected'] == expected, (case['id'], case['expected'], error)
        assert error.source.endswith('#' + case['source']), (case['id'], error.source, case['source'])
        assert error.instance_path == case['instancePath'], (case['id'], error.instance_path, case['instancePath'])
    else:
        assert case['expected'] == 'Valid', (case['id'], 'unexpected valid')

def example(name: str) -> dict[str, Any]:
    return copy.deepcopy(next(case['program'] for case in cases if case['id'] == name))

def instruction(program: dict[str, Any], op: str) -> dict[str, Any]:
    return next(check for node in program['nodes'] for check in node['checks'] if check['op'] == op)

guards: list[tuple[str, dict[str, Any], Callable[[dict[str, Any]], None]]] = []
base = example('if-then-selected')
guards.append(('unknown-version', base, lambda p: p.update(version='unknown')))
guards.append(('v3-profile', base, lambda p: p.update(version='suspect.validation.experimental.v3',profile='oas31-jsonschema202012-resource-dynamic')))
guards.append(('wrong-profile', base, lambda p: p.update(profile='oas31-jsonschema202012-static-subset')))
guards.append(('v2-in-v1', base, lambda p: p.update(version='suspect.validation.experimental.v1', profile='oas31-jsonschema202012-static-subset')))
guards.append(('unknown-op', base, lambda p: instruction(p, 'if').update(op='unimplemented')))
guards.append(('bad-target', base, lambda p: instruction(p, 'if').update(condition=-1)))
guards.append(('wrong-child-source', base, lambda p: instruction(p, 'if').update(thenTarget=instruction(p, 'if')['condition'])))
guards.append(('malformed-operand', base, lambda p: instruction(p, 'if').update(condition=True)))
guards.append(('unknown-operand', base, lambda p: instruction(p, 'if').update(ignored=True)))
guards.append(('duplicate-root', base, lambda p: p['roots'].append(copy.deepcopy(p['roots'][0]))))
guards.append(('invalid-limit', base, lambda p: p['limits'].update(maxEvaluationSteps=-1)))
guards.append(('invalid-source', base, lambda p: p['nodes'][0]['source'].update(pointer='/~2bad')))
pattern = example('pattern-overlap-accepts-and-excludes-extra')
guards.append(('pattern-version', pattern, lambda p: instruction(p, 'patternProperties')['patterns'][0][1].update(version='unknown')))
guards.append(('pattern-aware-required', pattern, lambda p: instruction(p, 'additionalPropertiesWithPatterns').update(op='additionalProperties')))
guards.append(('pattern-declared-names', pattern, lambda p: instruction(p, 'additionalPropertiesWithPatterns').update(declared=['invented'])))
contains = example('contains-marks-all-matches')
guards.append(('fractional-contains-count', contains, lambda p: instruction(p, 'contains').update(minimum='1.5')))
guards.append(('negative-contains-count', contains, lambda p: instruction(p, 'contains').update(maximum='-1')))
guards.append(('unknown-count-type', contains, lambda p: instruction(p, 'contains').update(minimum=1)))
unevaluated = example('required-is-not-an-evaluation')
guards.append(('unevaluated-order', unevaluated, lambda p: next(node['checks'] for node in p['nodes'] if any(check['op'] == 'unevaluatedProperties' for check in node['checks'])).reverse()))
dependent = example('dependent-required-null-is-present')
guards.append(('duplicate-dependent-name', dependent, lambda p: instruction(p, 'dependentRequired')['dependencies'][0][1].append('billing')))

for name, original, mutate in guards:
    program = copy.deepcopy(original)
    mutate(program)
    try:
        ValidationSession(program)
    except ProgramError:
        pass
    else:
        raise AssertionError(('malformed program admitted', name))

# Noninvertible shared failure and budgets are retained across root checks.
program = example('annotation-merges-use-shared-work')
session = ValidationSession(program)
for _ in range(2):
    try:
        session.check(program['roots'][0]['target'].to_int(), parse_json('{"a":1}'))
    except ValidationError as error:
        assert error.kind == 'evaluation_failure'
    else:
        raise AssertionError('root/trial reset a depleted work budget')
print(json.dumps({'vectors': len(cases), 'maintained_source_vectors': 32, 'guards': len(guards), 'shared_budget_checks': 2}))

# Caller-owned metadata mutation cannot bypass a session's checked program.
program = example('property-names-checks-key-not-value')
session = ValidationSession(program)
instruction(program, 'count')['value'] = '100'
try:
    session.check(program['roots'][0]['target'].to_int(), parse_json('{"long":1}'))
except ValidationError as error:
    assert error.kind == 'invalid' and error.instance_path == '/long'
else:
    raise AssertionError('caller mutation changed admitted program semantics')

# The 512 schema-depth budget trips on the explicit driver, not Python's stack.
program = example('productive-deep-object')
value: dict[str, Any] = {}
for _ in range(600):
    value = {'next': value}
before = sys.getrecursionlimit()
try:
    ValidationSession(program).check(program['roots'][0]['target'].to_int(), value)
except ValidationError as error:
    assert error.kind == 'evaluation_failure' and 'depth' in error.message, error
    assert error.source.endswith('#/components/schemas/Root'), error
    assert error.instance_path == '/next' * 256, error
else:
    raise AssertionError('deep schema exceeded its admitted evaluation depth')
assert sys.getrecursionlimit() == before
print('PROGRAM_SNAPSHOT_AND_NORMAL_STACK_DEPTH_PASSED')
"#;
