//! Explicit environment defaults through source-bound TypeScript SDK plans.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::credential_env::{CredentialEnv, CredentialEnvKind};
use suspect_codegen::{
    OutFile,
    typescript::{
        http::{HttpConfig, HttpPlan, plan_http},
        package::{PackageConfig, emit_http},
    },
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://source.example.test/credential-client/openapi.json";
const PACKAGE: &str = "@example/credential-client";

fn fixture() -> Value {
    let mut paths = serde_json::Map::new();
    for (path, operation, security) in [
        ("/bearer", "bearerOnly", json!([{"bearer":[]}])),
        ("/header", "headerOnly", json!([{"headerKey":[]}])),
        ("/query", "queryOnly", json!([{"queryKey":[]}])),
        ("/cookie", "cookieOnly", json!([{"cookieKey":[]}])),
        ("/either", "either", json!([{"bearer":[]},{"headerKey":[]}])),
        ("/both", "both", json!([{"bearer":[],"headerKey":[]}])),
        ("/anonymous", "anonymous", json!([])),
        ("/optional", "optional", json!([{}, {"bearer":[]}])),
    ] {
        paths.insert(path.into(), json!({"get":{"operationId":operation,"security":security,"responses":{"200":{"description":"credential fixture","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Result"}}}}}}}));
    }
    json!({"openapi":"3.2.0","info":{"title":"Source-bound environment credentials","version":"1"},
        "servers":[{"url":"https://api.example.test/v1"}],"paths":paths,
        "components":{"securitySchemes":{
            "bearer":{"type":"http","scheme":"bearer"},
            "headerKey":{"type":"apiKey","in":"header","name":"X-Key"},
            "queryKey":{"type":"apiKey","in":"query","name":"access_key"},
            "cookieKey":{"type":"apiKey","in":"cookie","name":"sid"}
        },"schemas":{"Result":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}},"additionalProperties":false}}}
    })
}

fn load_at(document: Value, physical: &str) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            Uri::parse(ENTRY).unwrap(),
            Uri::parse(physical).unwrap(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
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

fn plan(contract: Arc<Contract>, config: HttpConfig) -> HttpPlan {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    plan_http(contract, &selected, config).unwrap()
}

fn package(plan: &HttpPlan) -> Vec<OutFile> {
    emit_http(
        plan,
        &PackageConfig {
            name: PACKAGE.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap()
}

fn hashes(files: &[OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                format!("{:x}", Sha256::digest(file.content.as_bytes())),
            )
        })
        .collect()
}

#[test]
fn no_policy_http_package_keeps_its_baseline_bytes() {
    let files = package(&plan(load_at(fixture(), ENTRY), HttpConfig::expanded()));
    let mut expected: BTreeMap<String, String> = serde_json::from_str(include_str!(
        "fixtures/typescript-credential-env-no-policy-v1.json"
    ))
    .unwrap();
    // Explicit metadata type references preserve precision at full-contract size.
    assert_eq!(
        expected.insert(
            "typescript/operations.ts".into(),
            "c052325fd83eb7ffcce56e80127004a9475074dd90abd935c9b1e0036ea88b87".into()
        ),
        Some("1139268993b71885c25a01fd18c994b6a452bd1aa246c4e2379f6d3647e1478f".into())
    );
    assert_eq!(hashes(&files), expected, "no-policy artifact bytes changed");
}

fn environment_policy() -> CredentialEnv {
    CredentialEnv::v1(
        [
            ("bearer", "SUSPECT_TEST_BEARER"),
            ("headerKey", "SUSPECT_TEST_HEADER"),
            ("queryKey", "SUSPECT_TEST_QUERY"),
            ("cookieKey", "SUSPECT_TEST_COOKIE"),
        ]
        .map(|(name, variable)| (name.into(), variable.into()))
        .into(),
    )
}

#[test]
fn environment_plan_retains_source_binding_and_semantic_metadata() {
    let config = HttpConfig {
        credential_env: Some(environment_policy()),
        ..HttpConfig::expanded()
    };
    let planned = plan(load_at(fixture(), ENTRY), config.clone());
    let policy = planned.credential_env().unwrap();
    assert_eq!(policy.bindings().len(), 4);
    let bearer = policy
        .bindings()
        .iter()
        .find(|binding| binding.name() == "bearer")
        .unwrap();
    assert_eq!(bearer.kind(), CredentialEnvKind::Bearer);
    assert_eq!(bearer.variable(), "SUSPECT_TEST_BEARER");
    assert_eq!(
        bearer.scheme().use_site().source().pointer(),
        "/components/securitySchemes/bearer"
    );
    assert!(!bearer.scheme().use_site().span().is_empty());
    let relocated = plan(
        load_at(fixture(), "https://relocated.example.test/api.json"),
        config,
    );
    assert_eq!(
        policy.semantic_descriptor(),
        relocated.credential_env().unwrap().semantic_descriptor()
    );
    assert_ne!(
        bearer.scheme().use_site().source().document(),
        relocated
            .credential_env()
            .unwrap()
            .bindings()
            .iter()
            .find(|binding| binding.name() == "bearer")
            .unwrap()
            .scheme()
            .use_site()
            .source()
            .document()
    );
    assert!(
        planned
            .operations()
            .iter()
            .all(|operation| operation.interface()["constructor"]["optionsRequired"] == false)
    );
    let files = package(&planned);
    assert!(
        files
            .iter()
            .any(|file| file.path == "typescript/http/credential-env.ts")
    );
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "typescript/http-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        manifest["credentialEnv"]["bindings"][0]["variable"],
        "SUSPECT_TEST_BEARER"
    );
    assert!(files.iter().all(|file| {
        !file
            .content
            .contains("GENERATION_ONLY_CANARY_NOT_A_CREDENTIAL")
    }));
}

#[test]
fn environment_binding_refuses_invalid_unbound_and_unsupported_policies() {
    for (document, policy, code) in [
        (
            fixture(),
            CredentialEnv::v1(BTreeMap::new()),
            "sdk-credential-env-config",
        ),
        (
            fixture(),
            CredentialEnv::v1([("bearer".into(), "bad-name".into())].into()),
            "sdk-credential-env-config",
        ),
        (
            fixture(),
            CredentialEnv::v1([("unselected".into(), "VALID_NAME".into())].into()),
            "sdk-credential-env-unbound",
        ),
        (
            {
                let mut value = fixture();
                value["components"]["securitySchemes"]["bearer"] =
                    json!({"type":"http","scheme":"basic"});
                value
            },
            CredentialEnv::v1([("bearer".into(), "VALID_NAME".into())].into()),
            "sdk-credential-env-kind",
        ),
        (
            {
                let mut value = fixture();
                value["components"]["securitySchemes"]["bearer"] = json!({"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"https://auth.example.test/token","scopes":{}}}});
                value
            },
            CredentialEnv::v1([("bearer".into(), "VALID_NAME".into())].into()),
            "sdk-credential-env-kind",
        ),
        (
            {
                let mut value = fixture();
                value["components"]["securitySchemes"]["bearer"] = json!({"type":"openIdConnect","openIdConnectUrl":"https://auth.example.test/discovery"});
                value
            },
            CredentialEnv::v1([("bearer".into(), "VALID_NAME".into())].into()),
            "sdk-credential-env-kind",
        ),
    ] {
        let contract = load_at(document, ENTRY);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let errors = plan_http(
            contract,
            &selected,
            HttpConfig {
                credential_env: Some(policy),
                ..HttpConfig::expanded()
            },
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|finding| finding.code == code && !finding.at.is_empty()),
            "{code}: {errors:?}"
        );
    }
}

fn working_directory(label: &str) -> (PathBuf, Option<tempfile::TempDir>) {
    if let Some(parent) = std::env::var_os("SUSPECT_CREDENTIAL_ENV_ARTIFACTS") {
        std::fs::create_dir_all(&parent).unwrap();
        let root = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(parent)
            .unwrap()
            .keep();
        println!("retained credential-env evidence: {}", root.display());
        (root, None)
    } else {
        let temporary = tempfile::tempdir().unwrap();
        (temporary.path().to_owned(), Some(temporary))
    }
}

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn installed(label: &str, planned: &HttpPlan, types: &str, script: &str) {
    let (root, _temporary) = working_directory(label);
    suspect_codegen::write_files(&package(planned), &root).unwrap();
    let package = root.join("typescript");
    std::fs::write(
        root.join("bound-policy.json"),
        serde_json::to_vec_pretty(planned.credential_env().unwrap()).unwrap(),
    )
    .unwrap();
    let selected_node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let executable = checked(
        Command::new(selected_node).args(["--print", "process.execPath"]),
        &root,
    );
    let node = PathBuf::from(String::from_utf8(executable.stdout).unwrap().trim());
    let node24 = std::env::var_os("SUSPECT_NODE24_BIN")
        .expect("set SUSPECT_NODE24_BIN for required Node 24 execution");
    for (node, major) in [(node.as_os_str(), "22"), (node24.as_os_str(), "24")] {
        let version = checked(
            Command::new(node).args(["--print", "process.versions.node.split('.')[0]"]),
            &root,
        );
        assert_eq!(String::from_utf8_lossy(&version.stdout).trim(), major);
    }
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let npm_command = |cwd: &Path| {
        let mut command = Command::new(&node);
        command.arg(&npm).current_dir(cwd);
        command.env(
            "PATH",
            std::env::join_paths(std::iter::once(node.parent().unwrap().to_owned()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap(),
        );
        command
    };
    checked(
        npm_command(&package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &root,
    );
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools");
    let floor = root.join("floor");
    std::fs::create_dir(&floor).unwrap();
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(tools.join("typescript-floor").join(file), floor.join(file)).unwrap();
    }
    checked(
        npm_command(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &root,
    );
    for (version, compiler) in [
        ("5.9.3", package.join("node_modules/typescript/bin/tsc")),
        ("5.5.4", floor.join("node_modules/typescript/bin/tsc")),
    ] {
        let actual = checked(Command::new(&node).arg(&compiler).arg("--version"), &root);
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout).trim(),
            format!("Version {version}")
        );
        checked(
            Command::new(&node)
                .arg(&compiler)
                .current_dir(&package)
                .args(["--project", "tsconfig.json"]),
            &root,
        );
        if version == "5.9.3" {
            checked(
                Command::new(&node)
                    .arg(tools.join("typescript-docs/build.mjs"))
                    .arg(&package),
                &root,
            );
        }
        let packed = checked(
            npm_command(&package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
            &root,
        );
        let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
        let archive = root.join(format!("credential-client-ts{version}.tgz"));
        std::fs::rename(
            package.join(packed[0]["filename"].as_str().unwrap()),
            &archive,
        )
        .unwrap();
        let consumer = root.join(format!("installed-{version}"));
        std::fs::create_dir(&consumer).unwrap();
        std::fs::write(
            consumer.join("package.json"),
            "{\"type\":\"module\",\"private\":true}",
        )
        .unwrap();
        checked(
            npm_command(&consumer)
                .args([
                    "install",
                    "--offline",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                ])
                .arg(&archive),
            &root,
        );
        std::fs::write(consumer.join("consumer.ts"), types).unwrap();
        std::fs::write(consumer.join("consumer.mjs"), script).unwrap();
        checked(
            Command::new(&node)
                .arg(&compiler)
                .current_dir(&consumer)
                .args([
                    "--strict",
                    "--exactOptionalPropertyTypes",
                    "--noUncheckedIndexedAccess",
                    "--target",
                    "ES2022",
                    "--module",
                    "NodeNext",
                    "--moduleResolution",
                    "NodeNext",
                    "--noEmit",
                    "consumer.ts",
                ]),
            &root,
        );
        for executable in [node.as_os_str(), node24.as_os_str()] {
            checked(
                Command::new(executable)
                    .current_dir(&consumer)
                    .arg("node_modules/@example/credential-client/dist/examples/validated.js"),
                &root,
            );
            let output = checked(
                Command::new(executable)
                    .current_dir(&consumer)
                    .arg("consumer.mjs"),
                &root,
            );
            println!(
                "TypeScript {version}: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
        }
    }
}

#[test]
#[ignore = "requires pinned Node 22/24, offline TypeScript 5.5/5.9 and TypeDoc"]
fn installed_environment_defaults_preserve_creation_snapshot_explicit_auth_and_security_choices() {
    let mut document = fixture();
    document["paths"]["/collision"] = document["paths"]["/anonymous"].clone();
    document["paths"]["/collision"]["get"]["operationId"] = json!("__suspectCredentialEnv");
    let planned = plan(
        load_at(document, ENTRY),
        HttpConfig {
            credential_env: Some(environment_policy()),
            ..HttpConfig::expanded()
        },
    );
    installed("environment-", &planned, ENV_TYPES, ENV_RUNTIME);
}

const ENV_TYPES: &str = r#"
import {createClient} from '@example/credential-client';
import type {ClientOptions} from '@example/credential-client/operations';
const omitted=createClient();const empty=createClient({});
const policy:ClientOptions={};
const explicit=createClient({auth:{bearer:'token',headerKey:'header',queryKey:'query',cookieKey:'cookie'}});
// @ts-expect-error an explicit credentials object must meet its native credential contract
createClient({auth:{bearer:'partial'}});
// @ts-expect-error undefined is explicit and cannot stand in for omitted auth
createClient({auth:undefined});
// @ts-expect-error null is not the native credentials object
createClient({auth:null});
// @ts-expect-error credential structures cannot replace bearer/API-key strings
createClient({auth:{bearer:{token:'x'},headerKey:'h',queryKey:'q',cookieKey:'c'}});
export async function typedCalls(){const result=await omitted.bearerOnly();const ok:boolean=result.data.ok;await empty.anonymous();await empty.__suspectCredentialEnv();return ok;}
"#;

const ENV_RUNTIME: &str = r#"
import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
const processObject=globalThis.process,environmentDescriptor=Object.getOwnPropertyDescriptor(processObject,'env'),originalFetch=globalThis.fetch;
const values=Object.create(null),reads=[],requests=[];
const names=['SUSPECT_TEST_BEARER','SUSPECT_TEST_HEADER','SUSPECT_TEST_QUERY','SUSPECT_TEST_COOKIE'];
let unavailable;
const environment=new Proxy(Object.create(null),{get(_target,key){if(names.includes(key)){reads.push(key);if(key===unavailable)throw new Error('unavailable-value-must-not-leak');return values[key];}return undefined;}});
const setEnvironment=()=>Object.defineProperty(processObject,'env',{...environmentDescriptor,value:environment});
setEnvironment();
globalThis.fetch=async(input,init)=>{requests.push({url:String(input),headers:new Headers(init.headers)});return new Response('{"ok":true}',{status:200,headers:{'content-type':'application/json'}});};
try{
    Object.assign(values,{SUSPECT_TEST_BEARER:'import-token',SUSPECT_TEST_HEADER:'import-header',SUSPECT_TEST_QUERY:'query +&',SUSPECT_TEST_COOKIE:'cookie +/='});
    const {createClient,operations}=await import('@example/credential-client');
    assert.deepEqual(reads,[],'package import must not read mapped variables');
    values.SUSPECT_TEST_BEARER='creation-token';
    const first=createClient();assert.equal(reads.length,4);
    values.SUSPECT_TEST_BEARER='later-token';
    await first.bearerOnly();assert.equal(requests.at(-1).url,'https://api.example.test/v1/bearer');assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer creation-token');assert.equal(reads.length,4,'calls do not reread the environment');
    const second=createClient({});await second.bearerOnly();assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer later-token');
    await second.headerOnly();assert.equal(requests.at(-1).headers.get('X-Key'),'import-header');
    await second.queryOnly();assert.equal(requests.at(-1).url,'https://api.example.test/v1/query?access_key=query%20%2B%26');
    // Use the source-declared explicit transport seam for Node Cookie attachment.
    const cookies=createClient({fetch:globalThis.fetch});await cookies.cookieOnly();assert.equal(requests.at(-1).headers.get('Cookie'),'sid=cookie%20%2B%2F%3D');
    await second.both();assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer later-token');assert.equal(requests.at(-1).headers.get('X-Key'),'import-header');
    await second.either();assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer later-token');assert.equal(requests.at(-1).headers.get('X-Key'),null);
    await second.either({}, {securityAlternative:1});assert.equal(requests.at(-1).headers.get('Authorization'),null);assert.equal(requests.at(-1).headers.get('X-Key'),'import-header');
    await second.optional();assert.equal(requests.at(-1).headers.get('Authorization'),null);
    await second.optional({}, {securityAlternative:1});assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer later-token');
    await second.__suspectCredentialEnv();assert.equal(requests.at(-1).url,'https://api.example.test/v1/collision');

    const rejected=async action=>{const count=requests.length;await assert.rejects(action(),error=>operations.isSdkError(error)&&error.kind==='request-validation'&&!String(error.stack).includes('later-token')&&!String(error.cause?.stack).includes('unavailable-value-must-not-leak'));assert.equal(requests.length,count);};
    const beforeExplicit=reads.length;
    const explicit=createClient({auth:{bearer:'explicit-token'}});
    await explicit.bearerOnly();assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer explicit-token');
    await rejected(()=>explicit.both());
    for(const auth of [undefined,{}, {bearer:undefined}, {bearer:''}, {headerKey:'explicit-header'}]) {
        const client=createClient({auth});await rejected(()=>client.bearerOnly());
    }
    for(const auth of [null,''])assert.throws(()=>createClient({auth}),error=>error instanceof TypeError&&!String(error).includes('later-token'));
    assert.equal(reads.length,beforeExplicit,'every explicit auth argument bypasses environment lookup');
    await rejected(()=>operations.bearerOnly({}));assert.equal(reads.length,beforeExplicit,'standalone operation functions never perform environment lookup');

    delete values.SUSPECT_TEST_BEARER;values.SUSPECT_TEST_HEADER='fallback-header';
    const missing=createClient();await missing.either();assert.equal(requests.at(-1).headers.get('X-Key'),'fallback-header');await rejected(()=>missing.both());
    values.SUSPECT_TEST_BEARER='';const emptyValue=createClient();await emptyValue.either();assert.equal(requests.at(-1).headers.get('X-Key'),'fallback-header');await rejected(()=>emptyValue.bearerOnly());
    unavailable='SUSPECT_TEST_BEARER';const partial=createClient();unavailable=undefined;await partial.either();assert.equal(requests.at(-1).headers.get('X-Key'),'fallback-header');
    for(const name of names)delete values[name];
    const anonymous=createClient();await anonymous.anonymous();await anonymous.optional();await rejected(()=>anonymous.bearerOnly());await rejected(()=>anonymous.optional({}, {securityAlternative:1}));
    Object.defineProperty(processObject,'env',{configurable:true,enumerable:true,get(){throw new Error('host-environment-value-must-not-leak');}});
    const unavailableHost=createClient();setEnvironment();values.SUSPECT_TEST_BEARER='now-available';
    await unavailableHost.anonymous();await rejected(()=>unavailableHost.bearerOnly());
    await first.bearerOnly();assert.equal(requests.at(-1).headers.get('Authorization'),'Bearer creation-token');
    const report={node:processObject.versions.node,requests:requests.length,snapshot:true,explicitPrecedence:true,orAndAnonymous:true,importReads:0,sourceHttps:true};
    writeFileSync(`credential-env-${processObject.versions.node}.json`,JSON.stringify(report,null,2)+'\n');
    console.log('credential-env-native-passed',JSON.stringify(report));
}finally{Object.defineProperty(processObject,'env',environmentDescriptor);globalThis.fetch=originalFetch;}
"#;

fn openrouter_plan() -> HttpPlan {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../openrouter-web"));
    let path = root.join("projects/docs/openapi/openapi.yaml");
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
        .filter(|operation| {
            matches!(
                operation.operation_id(),
                Some("getCurrentKey" | "getCredits")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(
        selected.len(),
        2,
        "tracked getCurrentKey/getCredits operations"
    );
    let config = HttpConfig {
        credential_env: Some(CredentialEnv::v1(
            [("apiKey".into(), "OPENROUTER_API_KEY".into())].into(),
        )),
        ..HttpConfig::expanded()
    };
    let plan = plan_http(contract, &selected, config).unwrap();
    let policy = plan.credential_env().unwrap();
    assert_eq!(policy.bindings().len(), 1);
    assert_eq!(policy.bindings()[0].kind(), CredentialEnvKind::Bearer);
    assert_eq!(
        policy.bindings()[0].scheme().terminal().source().pointer(),
        "/components/securitySchemes/apiKey"
    );
    for operation in plan.protocol().operations() {
        assert_eq!(operation.method().as_str(), "GET");
        assert_eq!(
            operation.servers().candidates()[0].template(),
            "https://openrouter.ai/api/v1"
        );
    }
    assert_eq!(
        plan.operations()
            .iter()
            .find(|operation| operation.operation_id == "getCurrentKey")
            .unwrap()
            .protocol()
            .path(),
        "/key"
    );
    plan
}

fn openrouter_responses(plan: &HttpPlan) -> String {
    use suspect_codegen::examples::{ExampleOrigin, ExampleRole};
    let responses: BTreeMap<_, _> = plan
        .examples()
        .operations()
        .iter()
        .map(|operation| {
            let example = operation
                .entries
                .iter()
                .find(|entry| {
                    entry.origin == ExampleOrigin::Declared
                        && matches!(entry.role, ExampleRole::Response { status: 200 })
                })
                .expect("actual source-declared 200 response example");
            (operation.operation_id.clone(), example.value.to_string())
        })
        .collect();
    serde_json::to_string(&responses).unwrap()
}

#[test]
#[ignore = "requires tracked OpenRouter, pinned Node 22/24, offline TypeScript 5.5/5.9 and TypeDoc"]
fn installed_openrouter_current_key_uses_source_bearer_env_and_default_https() {
    let plan = openrouter_plan();
    assert!(package(&plan).iter().all(|file| {
        !file
            .content
            .contains("GENERATION_ONLY_CANARY_NOT_A_CREDENTIAL")
    }));
    let script = OPENROUTER_RUNTIME.replace("__RESPONSES__", &openrouter_responses(&plan));
    installed("openrouter-", &plan, OPENROUTER_TYPES, &script);
}

const OPENROUTER_TYPES: &str = r#"
import {createClient, JsonNumber} from '@example/credential-client';
const client=createClient();const withOptions=createClient({});
const explicit=createClient({auth:{apiKey:'caller-token'}});
// @ts-expect-error explicit credentials are not supplemented from the configured variable
createClient({auth:{}});
// @ts-expect-error omission differs from explicit undefined
createClient({auth:undefined});
// @ts-expect-error bearer input remains a string/provider, not a credential structure
createClient({auth:{apiKey:{token:'x'}}});
export async function readCurrentKey(){
    const response=await client.getCurrentKey();const status:200=response.status;
    const label:string=response.data.data.label;
    const management:boolean=response.data.data.is_management_key;
    const remaining:JsonNumber|null=response.data.data.limit_remaining;
    return {status,label,management,remaining};
}
export async function explicitManagementMode(){return (await client.getCredits()).data.data.total_credits;}
"#;

const OPENROUTER_RUNTIME: &str = r#"
import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
const responses=__RESPONSES__;
const host=globalThis.process,descriptor=Object.getOwnPropertyDescriptor(host,'env'),originalFetch=globalThis.fetch;
let token='import-value',reads=0,unavailable=false;const requests=[];
Object.defineProperty(host,'env',{...descriptor,value:new Proxy(Object.create(null),{get(_target,key){if(key==='OPENROUTER_API_KEY'){reads++;if(unavailable)throw new Error('unavailable-environment-secret');return token;}return undefined;}})});
globalThis.fetch=async(input,init)=>{
    const url=String(input);assert.ok(['https://openrouter.ai/api/v1/key','https://openrouter.ai/api/v1/credits'].includes(url),'only source-declared HTTPS endpoints are captured');
    assert.equal(init.method,'GET');assert.equal(init.redirect,'error');
    requests.push({url,authorization:new Headers(init.headers).get('Authorization')});
    return new Response(responses[url.endsWith('/key')?'getCurrentKey':'getCredits'],{status:200,headers:{'content-type':'application/json'}});
};
try{
    const {createClient,operations}=await import('@example/credential-client');
    assert.equal(reads,0,'import does not access the credential environment');
    token='first-creation';const first=createClient();assert.equal(reads,1);
    token='second-creation';
    const current=await first.getCurrentKey();assert.equal(current.status,200);assert.equal(current.data.data.is_management_key,false);assert.equal(current.data.data.limit_remaining.toString(),'74.5');assert.equal(current.data.data.rate_limit.requests,1000n);
    assert.deepEqual(requests[0],{url:'https://openrouter.ai/api/v1/key',authorization:'Bearer first-creation'});
    assert.equal(reads,1,'operation calls do not reread environment variables');
    const second=createClient({});await second.getCurrentKey();assert.equal(requests.at(-1).authorization,'Bearer second-creation');
    const beforeExplicit=reads;await createClient({auth:{apiKey:'explicit-token'}}).getCurrentKey();assert.equal(requests.at(-1).authorization,'Bearer explicit-token');assert.equal(reads,beforeExplicit);
    const rejects=async action=>{const before=requests.length;await assert.rejects(action(),error=>operations.isSdkError(error)&&error.kind==='request-validation'&&!String(error.stack).includes('second-creation')&&!String(error.cause?.stack).includes('unavailable-environment-secret'));assert.equal(requests.length,before);};
    for(const auth of [undefined,{}, {apiKey:undefined}, {apiKey:''}, {apiKey:null}, {other:'not-a-binding'}])await rejects(()=>createClient({auth}).getCurrentKey());
    for(const auth of [null,''])assert.throws(()=>createClient({auth}),error=>error instanceof TypeError&&!String(error).includes('second-creation'));
    assert.equal(reads,beforeExplicit,'explicit undefined/null/empty/missing-member auth never falls back');
    for(token of [undefined,''])await rejects(()=>createClient().getCurrentKey());
    unavailable=true;const absent=createClient();unavailable=false;token='later-present';await rejects(()=>absent.getCurrentKey());
    await first.getCredits();assert.equal(requests.at(-1).url,'https://openrouter.ai/api/v1/credits');assert.equal(requests.at(-1).authorization,'Bearer first-creation');
    const report={node:host.versions.node,defaultOperation:'getCurrentKey',method:'GET',url:requests[0].url,status:current.status,decoded:true,snapshot:true,explicitPrecedence:true,controlledTransport:true,liveAccount:false};
    writeFileSync(`openrouter-credential-env-${host.versions.node}.json`,JSON.stringify(report,null,2)+'\n');
    console.log('openrouter-credential-env-passed',JSON.stringify(report));
}finally{Object.defineProperty(host,'env',descriptor);globalThis.fetch=originalFetch;}
"#;

#[test]
#[ignore = "requires tracked OpenRouter, pinned Node/TypeScript and isolated Chromium (SUSPECT_CHROMIUM)"]
fn browser_environment_absence_preserves_explicit_and_anonymous_source_clients() {
    let (root, _temporary) = working_directory("browser-");
    let synthetic = plan(
        load_at(fixture(), ENTRY),
        HttpConfig {
            credential_env: Some(environment_policy()),
            ..HttpConfig::expanded()
        },
    );
    let actual = openrouter_plan();
    for (directory, planned) in [("synthetic", &synthetic), ("openrouter", &actual)] {
        let destination = root.join(directory);
        std::fs::create_dir(&destination).unwrap();
        suspect_codegen::write_files(&package(planned), &destination).unwrap();
        let compiler = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tools/typescript-docs/node_modules/typescript/bin/tsc");
        let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
        checked(
            Command::new(node)
                .arg(compiler)
                .current_dir(destination.join("typescript"))
                .args(["--project", "tsconfig.json"]),
            &root,
        );
    }
    std::fs::write(root.join("responses.json"), openrouter_responses(&actual)).unwrap();
    std::fs::write(
        root.join("browser.mjs"),
        include_str!("fixtures/typescript-credential-env-browser.mjs"),
    )
    .unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let result = checked(
        Command::new(node).current_dir(&root).arg("browser.mjs"),
        &root,
    );
    println!("{}", String::from_utf8_lossy(&result.stdout).trim());
}
