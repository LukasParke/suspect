//! Native physical retrieval bases are independent of logical OpenAPI identities.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::python_http::{self, PackageConfig};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

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
#[ignore = "requires native Python 3.11/3.14 and cached wheel/mypy tools"]
fn installed_python_physical_document_servers_preserve_redirects_overrides_and_encoded_paths() {
    let root = tempfile::Builder::new()
        .prefix("sdk-python-document-servers-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap();
    let responses =
        json!({"200":{"description":"source bytes","content":{"application/octet-stream":{}}}});
    let documents = vec![
        (
            "https://download.example/latest.json",
            "https://cdn.example/specs/releases/api.json",
            json!({
                "openapi":"3.2.0","$self":"https://logical.example/catalog/api.json#description", "info":{"title":"Physical server witness","version":"1"},
                "servers":[{"url":"../service"}],
                "paths":{
                    "/items":{"$ref":"parts.json#/components/pathItems/Items"},
                    "/external":{"$ref":"parts.json#/components/pathItems/External"},
                    "/empty":{"$ref":"parts.json#/components/pathItems/Empty"},
                    "/variable":{"$ref":"parts.json#/components/pathItems/Variable"}
                }
            }),
        ),
        (
            "https://download.example/parts.json",
            "https://storage.example/artifacts/parts.json",
            json!({
                "openapi":"3.2.0","$self":"https://logical.example/catalog/parts.json", "info":{"title":"External declarations","version":"1"},
                "components":{"securitySchemes":{
                    "oauth":{"type":"oauth2","oauth2MetadataUrl":"./metadata","flows":{"clientCredentials":{"tokenUrl":"../token","scopes":{}}}},
                    "oidc":{"type":"openIdConnect","openIdConnectUrl":"./discovery"}
                },"pathItems":{
                    "Items":{"get":{"operationId":"physicalItems","security":[{"oauth":[]}],"responses":responses}},
                    "External":{"get":{"operationId":"externalServer","security":[{"oidc":[]}],"servers":[{"url":"../v2/%2e%2e/Api%2Fv1"}],"responses":responses}},
                    "Empty":{"get":{"operationId":"emptyServer","servers":[],"responses":responses}},
                    "Variable":{"get":{"operationId":"variableServer","servers":[{"url":"{base}","variables":{"base":{"default":"https://absolute.example/v1"}}}],"responses":responses}}
                }}
            }),
        ),
    ];
    fs::write(
        root.join("documents.json"),
        serde_json::to_vec_pretty(&documents).unwrap(),
    )
    .unwrap();
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec(value).unwrap(),
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
    let contract = Arc::new(
        Contract::from_workspace(&workspace, &Uri::parse(documents[0].0).unwrap()).unwrap(),
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = python_http::plan_http(contract, &selected, Default::default()).unwrap();
    let files = python_http::emit_http(
        &plan,
        &PackageConfig {
            name: "document-python-sdk".into(),
            version: "1.0.0".into(),
            import_name: "document_python_sdk".into(),
        },
    )
    .unwrap();
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "python/src/document_python_sdk/http-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    let op = manifest["protocol"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["operation_id"]["value"] == "physicalItems")
        .unwrap();
    assert_eq!(
        op["servers"]["candidates"][0]["document_base"]["source"]["document"],
        documents[0].1
    );
    suspect_codegen::write_files(&files, &root).unwrap();
    fs::write(root.join("consumer.py"), CONSUMER).unwrap();
    let tools = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-native-python-tools/bin/python");
    checked(
        Command::new(&tools)
            .args(["-m", "build", "--wheel", "--no-isolation"])
            .current_dir(root.join("python")),
        &root,
        "build",
    );
    let wheel = fs::read_dir(root.join("python/dist"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.extension().is_some_and(|ext| ext == "whl"))
        .unwrap();
    for version in ["3.11", "3.14"] {
        let venv = root.join(format!("venv-{version}"));
        checked(
            Command::new("uv")
                .args(["venv", "--offline", "--python", version])
                .arg(&venv),
            &root,
            &format!("venv-{version}"),
        );
        let python = venv.join("bin/python");
        checked(
            Command::new("uv")
                .args(["pip", "install", "--offline", "--python"])
                .arg(&python)
                .arg(&wheel)
                .arg("Sphinx==8.2.3"),
            &root,
            &format!("install-{version}"),
        );
        checked(
            Command::new(&python).arg(root.join("consumer.py")),
            &root,
            &format!("native-{version}"),
        );
        checked(
            Command::new(&tools)
                .args([
                    "-m",
                    "mypy",
                    "--strict",
                    "--no-incremental",
                    "--python-version",
                    version,
                    "--python-executable",
                ])
                .arg(&python)
                .arg("--cache-dir")
                .arg(root.join(format!("mypy-{version}")))
                .arg(root.join("consumer.py"))
                .arg(root.join("python/src/document_python_sdk")),
            &root,
            &format!("mypy-{version}"),
        );
        checked(
            Command::new(&python)
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
                .arg(root.join(format!("sphinx-{version}")))
                .current_dir(root.join("python")),
            &root,
            &format!("sphinx-{version}"),
        );
        let coverage: Value = serde_json::from_slice(
            &fs::read(root.join(format!("sphinx-{version}/coverage.json"))).unwrap(),
        )
        .unwrap();
        assert!(
            coverage["signatures"]["document_python_sdk.CredentialRequest.effective_server_url"]
                .as_str()
                .unwrap()
                .contains("str | None")
        );
    }
    println!(
        "Python physical-document native witness: {}",
        root.display()
    );
}

const CONSUMER: &str = r#"from __future__ import annotations
import asyncio
import json
import sys
from pathlib import Path
import httpx
import document_python_sdk as sdk
from document_python_sdk import Client, AsyncClient, SdkError, Authorization, CredentialRequest

assert Path(sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
expected = [
    ('cdn.example', b'/specs/service/items'),
    ('storage.example', b'/v2/%2e%2e/Api%2Fv1/external'),
    ('storage.example', b'/empty'),
    ('absolute.example', b'/v1/variable'),
    ('runtime.example', b'/service/items'),
    ('runtime.example', b'/v2/%2e%2e/Api%2Fv1/external'),
    ('explicit.example', b'/Api%2Fv1/external'),
    ('storage.example', b'/artifacts/a//b//variable'),
    ('runtime.example', b'/ui/a//b//variable'),
    ('cdn.example', b'/specs/service/items'),
    ('storage.example', b'/v2/%2e%2e/Api%2Fv1/external'),
]
seen: list[tuple[str, bytes]] = []
contexts: list[CredentialRequest] = []
def credential(context: CredentialRequest) -> Authorization:
    host, path = expected[len(seen)]
    base = 'https://' + host + path.rsplit(b'/',1)[0].decode('ascii')
    assert context.effective_server_url == base, (context, base)
    assert context.scheme_source.document == 'https://storage.example/artifacts/parts.json'
    if context.name == 'oauth':
        assert context.metadata_url == './metadata'
        assert context.flows[0].token_url == '../token'
    else:
        assert context.name == 'oidc' and context.discovery_url == './discovery'
    contexts.append(context)
    return Authorization('Example explicit')
auth = {'oauth':credential,'oidc':credential}
def handle(request: httpx.Request) -> httpx.Response:
    actual = request.url.host, request.url.raw_path
    assert actual == expected[len(seen)], (actual, expected[len(seen)])
    if request.url.path.endswith(('/items','/external')):
        assert request.headers['Authorization'] == 'Example explicit'
    seen.append(actual)
    return httpx.Response(200, headers={'content-type': 'application/octet-stream'}, content=b'ok')

with httpx.MockTransport(handle) as transport:
    with Client(transport=transport,auth=auth) as client:
        assert client.physical_items().data == b'ok'
        assert client.external_server().data == b'ok'
        assert client.empty_server().data == b'ok'
        assert client.variable_server().data == b'ok'
    with Client(transport=transport, auth=auth, document_url='https://runtime.example/ui/root.json') as client:
        client.physical_items()
        client.external_server()
    with Client(transport=transport, auth=auth, server_url='https://explicit.example/Api%2Fv1') as client:
        client.external_server()
    with Client(transport=transport, server_variables={'base':'a//b//'}) as client:
        client.variable_server()
    with Client(transport=transport, document_url='https://runtime.example/ui/root.json',server_variables={'base':'a//b//'}) as client:
        client.variable_server()

    # Malformed overrides fail before transport, including urlsplit's bracket and
    # port errors. A local document never supplies an invented HTTP origin.
    for override in ['https://host.example:bad/v1','https://[bad/v1','https://u:p@host.example/v1','https://host.example/%GG','\nhttps://host.example/v1']:
        with Client(transport=transport, auth=auth, server_url=override) as client:
            try:
                client.physical_items()
            except SdkError as error:
                assert error.kind == 'request-representation', error
            else:
                raise AssertionError(('malformed override admitted',override))
    with Client(transport=transport, auth=auth, document_url='file:///source/api.json') as client:
        try:
            client.physical_items()
        except SdkError as error:
            assert error.kind == 'request-representation'
        else:
            raise AssertionError('file source supplied an HTTP origin')

async def asynchronous() -> None:
    async def supplied(context: CredentialRequest) -> Authorization:
        return credential(context)
    with httpx.MockTransport(handle) as transport:
        async with AsyncClient(transport=transport,auth={'oauth':supplied,'oidc':supplied}) as client:
            assert (await client.physical_items()).data == b'ok'
            assert (await client.external_server()).data == b'ok'
asyncio.run(asynchronous())
assert seen == expected
assert len(contexts) == 7
print(json.dumps({'python':sys.version,'requests':len(seen),'invalid_url_controls':6,'credential_contexts':len(contexts),'physical_redirect_base':True,'logical_self_not_server_base':True,'encoded_and_empty_segments':True,'oauth_oidc_effective_server_base':True}))
"#;
