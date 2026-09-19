//! Customer-facing seam: build/install the unmodified wheel, typecheck the
//! exact README snippets, build Sphinx against that installation, and execute
//! native constructors and authenticated requests on httpx.MockTransport.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    python_http::{self, HttpConfig, HttpPlan, PackageConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const IMPORT: &str = "documented_python_sdk";
const PROSE: &str = "Source guidance: `value` </pre><img src=x onerror=alert(1)>\n\n.. include:: must-not-read\n\n:ref:`must-not-resolve`\u{2028}.. raw:: html\n\n   <script>must-not-run</script>";

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn directory(label: &str) -> PathBuf {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    fs::create_dir_all(&target).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("sdk-python-dx-{label}-"))
        .tempdir_in(target)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

fn checked(command: &mut Command, root: &Path, label: &str) -> Output {
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    fs::write(root.join(format!("{label}.stdout.log")), &output.stdout).unwrap();
    fs::write(root.join(format!("{label}.stderr.log")), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{command:?}\nretained: {}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-native-python-tools/bin/python")
}

fn generated(
    contract: Arc<Contract>,
    selected: &[suspect_ir::contract::SourceId],
    config: HttpConfig,
) -> (HttpPlan, Vec<OutFile>) {
    let plan = python_http::plan_http(contract, selected, config).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "documented-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: IMPORT.into(),
        },
    )
    .unwrap();
    (plan, files)
}

fn file<'a>(files: &'a [OutFile], path: &str) -> &'a str {
    &files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("missing {path}"))
        .content
}

fn number(token: &str) -> Value {
    serde_json::from_str(token).unwrap()
}

fn edge_document() -> Value {
    let branch = |bound: &str, value| {
        json!({
            "type":"object", "required":["kind","score"], "additionalProperties":false,
            "properties":{"kind":{"type":"string","const":"same"},"score":{"type":"integer",bound:value}},
        })
    };
    let request = json!({
        "type":"object", "required":["class","count","items","properties","payload","number","json","nullable"],
        "properties":{
            "class":{"type":"string"}, "count":{"type":"integer"},
            "items":{"type":"array","items":{"$ref":"#/components/schemas/Node"}},
            "properties":{"type":"object","required":["items"],"additionalProperties":false,"properties":{"items":{"type":"integer"}}},
            "payload":{"oneOf":[{"$ref":"#/components/schemas/Big"},{"$ref":"#/components/schemas/Small"}]},
            "number":{"type":"number"},"json":true,"nullable":{"type":["string","null"]},
            "optional":{"type":["string","null"]},"literal":{"type":"integer","enum":[1,2]},
        },
        "additionalProperties":{"type":"array","items":{"type":"number"}},
    });
    let example = json!({
        "class":"quotes: '\"\\\n雪\u{2028} # still a value",
        "count":number("9007199254740993.0"),
        "items":[{"label":"parent","child":{"label":"leaf"}}],
        "properties":{"items":number("1.20e2")},"payload":{"kind":"same","score":2},
        "number":number("1e-400"),"json":{"integer":42,"decimal":number("1.2500"),"negative_zero":number("-0"),"array":[null,true,"雪"]},
        "nullable":null,"optional":"kept","literal":number("1.0"),"extra \" values":[number("2.50"),2],
    });
    json!({
        "openapi":"3.1.0","info":{"title":"Native documentation","version":"1"},
        "servers":[{"url":"https://native.example.test/v1"}],"security":[{"bearer\"\\雪":[]}],
        "components":{"securitySchemes":{"bearer\"\\雪":{"type":"http","scheme":"bearer"}},"schemas":{
            "Big":branch("minimum",10),"Small":branch("maximum",5),
            "Node":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/Node"}}},
        }},
        "paths":{"/probe/{slot}":{"post":{
            "operationId":"createProbe","description":PROSE,
            "parameters":[{"name":"slot","in":"path","required":true,"schema":{"type":"string"},"example":"alpha/slash"},
                {"name":"client","in":"query","schema":{"type":"string"},"example":"query value"},
                {"name":"body","in":"query","schema":{"type":"string"},"example":"query-body"}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":request,"example":example}}},
            "responses":{
                "201":{"description":"Created","content":{"application/json":{"schema":{"type":"object","required":["accepted"],"properties":{"accepted":{"type":"boolean"}}},"example":{"accepted":true}}}},
                "401":{"description":"Denied","content":{"application/json":{"schema":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}},"example":{"message":"source-denied"}}}},
            },
        }}},
    })
}

#[test]
fn native_availability_retains_source_roles_union_proofs_and_limits() {
    let root = directory("plans");
    let path = root.join("quoted ' source.json");
    fs::write(&path, edge_document().to_string()).unwrap();
    let contract = contract(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let (_, files) = generated(contract.clone(), &selected, Default::default());
    let bindings: Value =
        serde_json::from_str(file(&files, "python/docs/source-bindings.json")).unwrap();
    assert_eq!(bindings["recipes"]["quickstart"]["method"], "create_probe");
    assert_eq!(
        bindings["recipes"]["presence"]["variants"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(bindings["examples"][0]["nativeAvailable"], true);
    let entries = bindings["examples"][0]["entries"].as_array().unwrap();
    let body = entries
        .iter()
        .find(|entry| entry["role"]["kind"] == "request-body")
        .unwrap();
    assert_eq!(body["origin"], "declared");
    assert_eq!(
        body["native"]["branches"][0]["pointer"],
        "/paths/~1probe~1{slot}/post/requestBody/content/application~1json/schema/properties/payload/oneOf/1"
    );
    assert_eq!(
        body["declaredSource"]["pointer"],
        "/paths/~1probe~1{slot}/post/requestBody/content/application~1json/example"
    );
    assert!(
        bindings["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |symbol| symbol["name"] == format!("{IMPORT}.operations.CreateProbeStatus201")
                    && symbol["file"] == format!("src/{IMPORT}/operations.py")
            )
    );
    for name in [
        "JsonNumber",
        "JsonValue",
        "CodecError",
        "JsonError",
        "ValidationError",
    ] {
        assert!(
            bindings["symbols"]
                .as_array()
                .unwrap()
                .iter()
                .any(|symbol| symbol["name"] == format!("{IMPORT}.{name}"))
        );
    }
    // Tight semantic policy is also used for native branch selection. An
    // incomplete evaluation is retained, not claimed as a usable construction.
    let mut config = HttpConfig::default();
    config.codecs.schema.max_evaluation_steps = 0;
    let (_, limited) = generated(contract, &selected, config);
    let bindings: Value =
        serde_json::from_str(file(&limited, "python/docs/source-bindings.json")).unwrap();
    assert_eq!(bindings["examples"][0]["available"], true);
    assert_eq!(bindings["examples"][0]["nativeAvailable"], false);
    assert_eq!(
        bindings["examples"][0]["entries"][0]["native"]["code"],
        "native-example-evaluation-incomplete"
    );
    assert_eq!(bindings["recipes"]["quickstart"]["available"], false);
    suspect_codegen::write_files(&files, &root).unwrap();
}

#[test]
fn unrepresentable_integer_examples_are_explicitly_unavailable() {
    let root = directory("integer-limit");
    let mut document = edge_document();
    document["paths"]["/probe/{slot}"]["post"]["requestBody"]["content"]["application/json"]["schema"] =
        json!({"type":"integer"});
    document["paths"]["/probe/{slot}"]["post"]["requestBody"]["content"]["application/json"]["example"] =
        number("1e5000");
    let path = root.join("api.json");
    fs::write(&path, document.to_string()).unwrap();
    let contract = contract(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let (_, files) = generated(contract, &selected, Default::default());
    let bindings: Value =
        serde_json::from_str(file(&files, "python/docs/source-bindings.json")).unwrap();
    assert_eq!(bindings["examples"][0]["available"], true);
    assert_eq!(bindings["examples"][0]["nativeAvailable"], false);
    let body = bindings["examples"][0]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["role"]["kind"] == "request-body")
        .unwrap();
    assert_eq!(body["native"]["code"], "native-example-integer-limit");
    assert_eq!(bindings["recipes"]["quickstart"]["available"], false);
}

#[test]
#[ignore = "requires pinned Python build/httpx/mypy/Sphinx tools and installed 3.11/3.14 interpreters"]
fn installed_m2_and_edge_quickstarts_are_public_native_and_typechecked() {
    for edge in [false, true] {
        let root = directory(if edge { "edge" } else { "m2" });
        let document = if edge {
            edge_document()
        } else {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/m2/canonical.openapi.yaml");
            let source = contract(&path);
            let mut value = source.document(source.entry()).unwrap().clone();
            value["paths"]["/widgets"]["post"]["description"] = json!(PROSE);
            value
        };
        let path = root.join("source ' quoted.json");
        fs::write(&path, document.to_string()).unwrap();
        let contract = contract(&path);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let (plan, files) = generated(contract, &selected, Default::default());
        let config = if edge {
            json!({"scenario":"edge","scheme":"bearer\"\\雪","count":1})
        } else {
            json!({"scenario":"m2","scheme":"apiKey","count":4})
        };
        native_gate(&root, &plan, &files, config);
    }
}

#[test]
#[ignore = "requires tracked OpenRouter source plus native Python 3.11/3.14 build/docs tools"]
fn installed_tracked_five_operation_quickstart_uses_real_constructors() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&path);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| wanted.contains(&op.operation_id().unwrap_or_default()))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let (plan, files) = generated(contract, &selected, Default::default());
    let root = directory("openrouter");
    native_gate(
        &root,
        &plan,
        &files,
        json!({"scenario":"openrouter","scheme":"apiKey","count":5}),
    );
}

fn native_gate(root: &Path, plan: &HttpPlan, files: &[OutFile], config: Value) {
    suspect_codegen::write_files(files, root).unwrap();
    let package = root.join("python");
    let bindings: Value =
        serde_json::from_str(file(files, "python/docs/source-bindings.json")).unwrap();
    assert!(
        bindings["examples"]
            .as_array()
            .unwrap()
            .iter()
            .all(|example| example["nativeAvailable"] == true),
        "{bindings}"
    );
    assert_eq!(bindings["recipes"]["quickstart"]["available"], true);
    let first = bindings["recipes"]["quickstart"]["method"]
        .as_str()
        .unwrap();
    let snippets = root.join("readme-snippets");
    fs::create_dir(&snippets).unwrap();
    let readme = file(files, "python/README.md");
    let mut count = 0;
    // Extract the exact fenced blocks at the documentation-consumer seam. The
    // generator itself never parses Python to discover names or signatures.
    for part in readme.split("```python\n").skip(1) {
        let snippet = part.split_once("\n```").unwrap().0.to_owned() + "\n";
        assert!(
            files
                .iter()
                .any(|file| file.path.starts_with("python/examples/") && file.content == snippet),
            "README example differs from its executable source"
        );
        fs::write(snippets.join(format!("snippet_{count}.py")), snippet).unwrap();
        count += 1;
    }
    assert!(count >= 5);
    fs::write(root.join("probe.py"), PROBE.replace("__IMPORT__", IMPORT)).unwrap();
    fs::write(root.join("probe-config.json"), config.to_string()).unwrap();
    checked(
        Command::new(tools())
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .env_remove("PYTHONPATH")
            .current_dir(&package),
        root,
        "build",
    );
    let wheel = fs::read_dir(package.join("dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let environment = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&environment),
            root,
            &format!("venv-{version}"),
        );
        let python = environment.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel),
            root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(package.join("examples"))
                .arg(&snippets)
                .arg(root.join("probe.py"))
                .env_remove("PYTHONPATH"),
            root,
            &format!("mypy-{version}"),
        );
        checked(
            Command::new(&python)
                .arg(package.join("examples/validated.py"))
                .env_remove("PYTHONPATH")
                .current_dir(root),
            root,
            &format!("examples-{version}"),
        );
        checked(
            Command::new(&python)
                .arg(root.join("probe.py"))
                .env_remove("PYTHONPATH")
                .current_dir(root),
            root,
            &format!("probe-{version}"),
        );
        let site = checked(
            Command::new(&python).args(["-c", "import site; print(site.getsitepackages()[0])"]),
            root,
            &format!("site-{version}"),
        );
        checked(
            Command::new(tools())
                .args([
                    "-m",
                    "sphinx",
                    "-W",
                    "--keep-going",
                    "-E",
                    "-b",
                    "html",
                    "docs",
                ])
                .arg(format!("docs/_build/{version}"))
                .env("PYTHONPATH", String::from_utf8(site.stdout).unwrap().trim())
                .current_dir(&package),
            root,
            &format!("sphinx-{version}"),
        );
        let coverage: Value = serde_json::from_slice(
            &fs::read(package.join(format!("docs/_build/{version}/coverage.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            coverage["plannedSymbols"].as_u64().unwrap() as usize,
            bindings["symbols"].as_array().unwrap().len()
        );
        let html =
            fs::read_to_string(package.join(format!("docs/_build/{version}/getting-started.html")))
                .unwrap();
        assert!(!html.contains("<img src=x") && !html.contains("<script>must-not-run"));
    }
    checked(
        Command::new(tools())
            .args([
                "-m",
                "mypy",
                "--strict",
                "--python-version",
                "3.11",
                "--cache-dir",
            ])
            .arg(root.join("package-mypy"))
            .arg(package.join(format!("src/{IMPORT}")))
            .env_remove("PYTHONPATH"),
        root,
        "package-mypy",
    );
    let native_count = bindings["examples"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|example| example["entries"].as_array().unwrap())
        .filter(|entry| entry["native"]["available"] == true)
        .count();
    fs::write(root.join("evidence.json"), serde_json::to_string_pretty(&json!({
        "package":IMPORT,"operations":plan.operations().len(),"nativeExamples":native_count,"readmeSnippets":count,
        "quickstart":first,"interpreters":["3.11","3.14"],"gates":["installed-wheel","strict-mypy-package","strict-mypy-readme-and-guides","native-vs-decoded-encode-bytes","authenticated-MockTransport","Sphinx-W-imported-symbols"],
    })).unwrap()).unwrap();
    println!("Python DX evidence: {}", root.display());
}

const PROBE: &str = r#"from __future__ import annotations

import asyncio
from contextlib import redirect_stdout
from decimal import Decimal
import importlib
import inspect
import io
import json
from pathlib import Path
import sys
from typing import Any, get_type_hints
from unittest.mock import patch

import httpx
import __IMPORT__ as sdk
from __IMPORT__ import (
    ApiError, Client, CodecError, JsonError, JsonNumber, JsonValue, SdkError,
    UNSET, Unset, ValidationError, models, operations,
)
from __IMPORT__ import _client as legacy
from __IMPORT__.codec_runtime import CodecError as InternalCodecError
from __IMPORT__.json_runtime import (
    JsonError as InternalJsonError, JsonNumber as InternalJsonNumber, stringify_json,
)
from __IMPORT__.validation import ValidationError as InternalValidationError

ROOT = Path(__file__).parent
PACKAGE = ROOT / 'python'
CONFIG = json.loads((ROOT / 'probe-config.json').read_text())
BINDINGS = json.loads((PACKAGE / 'docs/source-bindings.json').read_text())
EXAMPLES = json.loads((PACKAGE / 'src/__IMPORT__/examples.json').read_text(), parse_float=JsonNumber, parse_int=JsonNumber)
for example_operation in EXAMPLES['operations']:
    for example_entry in example_operation['entries']:
        if example_entry['role']['kind'] == 'response':
            example_entry['role']['status'] = example_entry['role']['status'].to_int()
assert Path(sdk.__file__ or '').is_relative_to(Path(sys.prefix)), 'must import installed wheel'
sys.path.insert(0, str(PACKAGE / 'examples'))
quickstart = importlib.import_module('quickstart')
asynchronous = importlib.import_module('async_client')
errors = importlib.import_module('errors')
presence = importlib.import_module('presence')
transport_recipe = importlib.import_module('transport')
validated = importlib.import_module('validated')

for snippet in (ROOT / 'readme-snippets').glob('*.py'):
    exec(compile(snippet.read_text(), str(snippet), 'exec'), {'__name__': 'readme_consumer'})

assert set(sdk.__all__) == {
    'Client', 'AsyncClient', 'ApiError', 'SdkError', 'CodecError', 'JsonError',
    'ValidationError', 'JsonNumber', 'JsonValue', 'UNSET', 'Unset', 'models', 'codecs', 'operations',
    'BasicAuth', 'Authorization', 'CredentialRequest', 'OAuthFlow', 'AuthValue',
    'Credential', 'CredentialProvider', 'Part', 'Link', 'SyncStream', 'AsyncStream',
}
assert JsonNumber is InternalJsonNumber
assert CodecError is InternalCodecError
assert JsonError is InternalJsonError
assert ValidationError is InternalValidationError
assert UNSET is models.UNSET and isinstance(UNSET, Unset) and UNSET is not None
exact: JsonValue = {'integer': 42, 'decimal': JsonNumber('1.2500')}
assert stringify_json(exact) == '{"integer":42,"decimal":1.2500}'
public_names = set(operations.__all__)
for operation in BINDINGS['operations']:
    assert operation['resultModule'] == '__IMPORT__.operations'
    for client_type in (sdk.Client, sdk.AsyncClient):
        assert get_type_hints(getattr(client_type, operation['method']))['return'] == getattr(operations, operation['success'])
    for name in [operation['success'], operation['apiError'], *(response['class'] for response in operation['responses'])]:
        assert name in public_names
        assert getattr(operations, name) is getattr(legacy, name)
    for response in operation['responses']:
        cls = getattr(operations, response['class'])
        assert cls.__module__ == '__IMPORT__.operations'
        if response['status'] >= 300:
            assert issubclass(cls, ApiError)

FIRST = BINDINGS['recipes']['quickstart']['method']
FIRST_OPERATION = next(op for op in BINDINGS['operations'] if op['method'] == FIRST)
FIRST_EXAMPLES = next(op for op in EXAMPLES['operations'] if op['source'] == FIRST_OPERATION['source'])
SUCCESS = next(entry for entry in FIRST_EXAMPLES['entries'] if entry['role']['kind'] == 'response' and 200 <= entry['role']['status'] < 300)
FAILURE = next(entry for entry in FIRST_EXAMPLES['entries'] if entry['role']['kind'] == 'response' and entry['role']['status'] >= 300)
seen: list[httpx.Request] = []
closed: list[str] = []
mode = 'success'


def handle(request: httpx.Request) -> httpx.Response:
    assert request.headers['Authorization'] == 'Bearer dx-token'
    assert request.headers['Accept'] == 'application/json'
    seen.append(request)
    if mode == 'transport':
        raise RuntimeError('secret-transport-cause')
    if mode == 'unexpected':
        return httpx.Response(599, headers={'Content-Type': 'application/json'}, content=b'"secret-capture"')
    if mode == 'failure':
        return httpx.Response(FAILURE['role']['status'], headers={'Content-Type': 'application/json'}, content=stringify_json(FAILURE['value']).encode())
    if CONFIG['scenario'] == 'm2':
        payload = b'{"items":[]}' if request.url.path == '/api/v1/widgets' and request.method == 'GET' else b'{"id":"w1","amount":9007199254740993.000000000000000001,"payload":{"kind":"standard","text":"plain"}}'
        return httpx.Response(200, headers={'Content-Type': 'application/json'}, content=payload)
    if CONFIG['scenario'] == 'edge':
        return httpx.Response(201, headers={'Content-Type': 'application/json'}, content=b'{"accepted":true}')
    # The real-source fixture uses the accepted wire response for each route.
    # Request assertions below are independent of that generated response data.
    if request.url.path == '/api/v1/keys' and request.method == 'POST':
        operation_id = 'createKeys'
    elif request.url.path.startswith('/api/v1/keys/'):
        operation_id = 'updateKeys'
    elif request.url.path == '/api/v1/credits':
        operation_id = 'getCredits'
    elif request.url.path.endswith('/files'):
        operation_id = 'listContainerFiles'
    else:
        operation_id = 'getContainerFile'
    operation = next(op for op in EXAMPLES['operations'] if op['operationId'] == operation_id)
    response = next(entry for entry in operation['entries'] if entry['role']['kind'] == 'response' and 200 <= entry['role']['status'] < 300)
    return httpx.Response(response['role']['status'], headers={'Content-Type': 'application/json'}, content=stringify_json(response['value']).encode())


class RecordingTransport(httpx.MockTransport):
    def __init__(self) -> None:
        super().__init__(handle)

    def close(self) -> None:
        closed.append('sync')
        super().close()

    async def aclose(self) -> None:
        closed.append('async')
        await super().aclose()


def factory(*args: object, **kwargs: object) -> RecordingTransport:
    assert kwargs == {'trust_env': False, 'retries': 0}
    return RecordingTransport()


def decoded(request: httpx.Request) -> Any:
    return json.loads(request.content, parse_float=Decimal)


def check_first(request: httpx.Request) -> None:
    if CONFIG['scenario'] == 'm2':
        assert request.method == 'POST' and request.url.raw_path == b'/api/v1/widgets'
        assert request.content == b'{"name":"alpha"}'
    elif CONFIG['scenario'] == 'edge':
        assert request.method == 'POST' and request.url.raw_path == b'/v1/probe/alpha%2Fslash?client=query%20value&body=query-body'
        body = decoded(request)
        assert body['count'] == 9007199254740993 and isinstance(body['count'], int)
        assert body['properties'] == {'items': 120}
        assert body['payload'] == {'kind': 'same', 'score': 2}
        assert body['items'] == [{'label': 'parent', 'child': {'label': 'leaf'}}]
        assert body['number'] == Decimal('1e-400')
        assert body['json']['decimal'] == Decimal('1.2500') and body['json']['integer'] == 42
        assert body['nullable'] is None and body['optional'] == 'kept'
        assert body['literal'] == 1 and isinstance(body['literal'], int)
        assert body['extra " values'] == [Decimal('2.50'), 2]
        assert body['class'] == "quotes: '\"\\\n雪\u2028 # still a value"
    else:
        assert FIRST == 'create_keys'
        assert request.method == 'POST' and request.url.raw_path == b'/api/v1/keys'
        assert decoded(request) == {'expires_at': '2027-12-31T23:59:59Z', 'include_byok_in_limit': True, 'limit': 50, 'limit_reset': 'monthly', 'name': 'My New API Key'}


with patch('httpx.HTTPTransport', side_effect=factory), patch('httpx.AsyncHTTPTransport', side_effect=factory):
    response = quickstart.first_request('dx-token')
    assert response.status == SUCCESS['role']['status']
    check_first(seen[-1])
    assert closed == ['sync']
    response = asyncio.run(asynchronous.first_request_async('dx-token'))
    check_first(seen[-1])
    assert closed == ['sync', 'async']
    for failure_mode in ('failure', 'unexpected', 'transport'):
        mode = failure_mode
        output = io.StringIO()
        with redirect_stdout(output):
            assert errors.request_with_errors('dx-token') is None
        assert 'secret' not in output.getvalue()
    mode = 'success'
    start = len(seen)
    with Client(auth={CONFIG['scheme']: 'dx-token'}) as client:
        validated.run_sync(client)
    assert len(seen) - start == CONFIG['count']
    if CONFIG['scenario'] == 'm2':
        assert {(r.method, r.url.raw_path, r.content) for r in seen[start:]} == {
            ('POST', b'/api/v1/widgets', b'{"name":"alpha"}'),
            ('GET', b'/api/v1/widgets?tag=x&limit=1', b''),
            ('GET', b'/api/v1/widgets/x', b''),
            ('PATCH', b'/api/v1/widgets/x', b'{}'),
        }
        with Client(auth={'apiKey': 'dx-token'}) as client:
            before = len(seen)
            try:
                getattr(client, 'create_widget')(body=getattr(models, 'WidgetInput')(name=''))
            except CodecError:
                pass
            else:
                raise AssertionError('invalid native input reached transport')
            assert len(seen) == before
    async def run_async() -> None:
        async with sdk.AsyncClient(auth={CONFIG['scheme']: 'dx-token'}) as client:
            await validated.run_async(client)
    start = len(seen)
    asyncio.run(run_async())
    assert len(seen) - start == CONFIG['count']
    variants = presence.variants()
    with Client(auth={CONFIG['scheme']: 'dx-token'}) as client:
        start = len(seen)
        presence.send_variants(client)
    assert len(seen) - start == len(variants)
    wire = BINDINGS['recipes']['presence']['wire']
    assert wire not in decoded(seen[start])
    if len(variants) == 3:
        assert decoded(seen[start + 1])[wire] is None
        assert decoded(seen[start + 2])[wire] is not None
    if CONFIG['scenario'] == 'openrouter':
        assert wire == 'limit'
        assert decoded(seen[start + 2])['limit'] == 75
    custom = RecordingTransport()
    before = len(closed)
    transport_recipe.with_transport('dx-token', custom)
    check_first(seen[-1])
    assert seen[-1].extensions['timeout'] == {'connect': 10.0, 'read': 10.0, 'write': 10.0, 'pool': 10.0}
    assert len(closed) == before, 'client closed a caller-owned transport'
    custom.close()
    assert len(closed) == before + 1

transport_recipe.mock_request()
for operation in BINDINGS['examples']:
    for entry in operation['entries']:
        if entry['role']['kind'] == 'request-body' and entry['native']['available']:
            native = getattr(validated, entry['native']['function'])()
            if CONFIG['scenario'] == 'edge':
                assert type(native.payload) is getattr(models, 'Small')
                assert 'kind' not in inspect.signature(getattr(models, 'Small')).parameters
                assert native.count == 9007199254740993
                assert native.json['integer'] == 42 and type(native.json['integer']) is int
                assert type(native.json['decimal']) is JsonNumber

print(json.dumps({'python': sys.version.split()[0], 'scenario': CONFIG['scenario'], 'requests': len(seen), 'public_operation_exports': len(public_names), 'owned_transport_closes': len(closed)}))
"#;
