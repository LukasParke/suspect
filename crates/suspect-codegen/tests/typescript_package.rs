//! ESM package resolution and codec behavior through installed local tarballs.

use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    typescript::{
        codecs::{CodecConfig, CodecPlan, plan_codecs},
        http::{HttpConfig, plan_http},
        package::{PackageConfig, PackageError, emit, emit_http},
    },
};
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

fn plan(schemas: Value) -> CodecPlan {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(
        &path,
        json!({"openapi":"3.1.0", "info":{"title":"Package","version":"1"},
            "paths":{}, "components":{"schemas":schemas}})
        .to_string(),
    )
    .unwrap();
    let contract = load(&path);
    plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        CodecConfig::default(),
    )
    .unwrap()
}

fn config() -> PackageConfig {
    PackageConfig {
        name: "@suspect-fixtures/model-codecs".into(),
        version: "0.1.0-test.1".into(),
    }
}

fn content<'a>(files: &'a [OutFile], path: &str) -> &'a str {
    &files.iter().find(|file| file.path == path).unwrap().content
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn package_identity_and_version_are_validated_before_emission() {
    let plan = plan(json!({"Flag":{"type":"boolean"}}));
    for name in [
        "",
        "Upper",
        " has-space",
        "api ",
        "../api",
        "a/b",
        "@/name",
        "@scope/",
        "@scope/a/b",
        "@Scope/name",
        ".hidden",
        "_hidden",
        "with%20escape",
        "with~tilde",
        "node_modules",
        "favicon.ico",
        "fs",
        "http",
        "process",
        "@scope/雪",
        "line\nfeed",
    ] {
        assert_eq!(
            emit(
                &plan,
                &PackageConfig {
                    name: name.into(),
                    ..config()
                }
            ),
            Err(PackageError::InvalidName),
            "{name:?}",
        );
    }
    assert_eq!(
        emit(
            &plan,
            &PackageConfig {
                name: "a".repeat(215),
                ..config()
            }
        ),
        Err(PackageError::InvalidName),
    );
    for version in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "v1.2.3",
        "=1.2.3",
        "^1.2.3",
        "~1.2.3",
        "1.2.*",
        " 1.2.3",
        "1.2.3 ",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3-",
        "1.2.3+",
        "1.2.3-a..b",
        "1.2.3-01",
        "1.2.3-alpha.01",
        "1.2.3-a_b",
        "1.2.3+build+again",
        "9007199254740992.0.0",
        "1.2.3-雪",
    ] {
        assert_eq!(
            emit(
                &plan,
                &PackageConfig {
                    version: version.into(),
                    ..config()
                }
            ),
            Err(PackageError::InvalidVersion),
            "{version:?}",
        );
    }
    assert_eq!(
        emit(
            &plan,
            &PackageConfig {
                version: format!("1.2.3+{}", "a".repeat(251)),
                ..config()
            }
        ),
        Err(PackageError::InvalidVersion),
    );
    for (name, version) in [
        ("api", "0.0.0"),
        ("@scope/fs", "1.2.3-alpha.0+build.001"),
        ("api.with_under-score", "9007199254740991.0.0"),
        ("0", "1.2.3-9999999999999999999999999999"),
    ] {
        assert!(
            emit(
                &plan,
                &PackageConfig {
                    name: name.into(),
                    version: version.into()
                }
            )
            .is_ok()
        );
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn package_preserves_plan_artifacts_and_declares_bounded_pinned_esm_output() {
    let plan = plan(json!({"Flag":{"type":"boolean","description":"Original schema prose."}}));
    let files = emit(&plan, &config()).unwrap();
    assert_eq!(files, emit(&plan, &config()).unwrap());
    for source in plan.render() {
        assert_eq!(content(&files, &source.path), source.content);
    }
    assert_eq!(files.len(), plan.render().len() + 5);
    let metadata_text = content(&files, "typescript/package.json");
    let metadata: Value = serde_json::from_str(metadata_text).unwrap();
    assert_eq!(metadata["name"], config().name);
    assert_eq!(metadata["version"], config().version);
    assert_eq!(metadata["private"], true);
    assert_eq!(metadata["type"], "module");
    assert_eq!(metadata["engines"], json!({"node":">=22"}));
    assert_eq!(metadata["packageManager"], "npm@10.9.8");
    assert_eq!(metadata["devDependencies"], json!({"typescript":"5.9.3"}));
    assert_eq!(
        metadata["scripts"],
        json!({"build":"tsc --project tsconfig.json"})
    );
    for key in [
        "dependencies",
        "peerDependencies",
        "optionalDependencies",
        "sideEffects",
    ] {
        assert!(
            metadata.get(key).is_none(),
            "unexpected package claim: {key}"
        );
    }
    assert_eq!(metadata["suspect"]["modelView"], "Neutral");
    assert_eq!(metadata["suspect"]["status"], "prototype");
    assert_eq!(metadata["suspect"]["releaseReady"], false);
    assert_eq!(metadata["suspect"]["httpClient"], false);
    assert!(
        metadata_text
            .contains("\"types\": \"./dist/models.d.ts\",\n      \"import\": \"./dist/models.js\"")
    );
    assert_eq!(
        metadata["exports"]["./codecs"]["import"],
        "./dist/model-codecs.js"
    );
    let lock: Value =
        serde_json::from_str(content(&files, "typescript/package-lock.json")).unwrap();
    let reviewed: Value =
        serde_json::from_str(include_str!("../tools/typescript-docs/package-lock.json")).unwrap();
    assert_eq!(lock["packages"].as_object().unwrap().len(), 2);
    assert_eq!(
        lock["packages"]["node_modules/typescript"],
        reviewed["packages"]["node_modules/typescript"]
    );
    assert_eq!(
        lock["packages"][""]["devDependencies"],
        metadata["devDependencies"]
    );
    let build: Value = serde_json::from_str(content(&files, "typescript/tsconfig.json")).unwrap();
    assert_eq!(build["compilerOptions"]["target"], "ES2022");
    assert_eq!(build["compilerOptions"]["module"], "NodeNext");
    assert_eq!(build["compilerOptions"]["declaration"], true);
    assert_eq!(build["compilerOptions"]["noEmitOnError"], true);
    assert_eq!(build["files"], json!(["source/index.ts"]));
    let readme = content(&files, "typescript/README.md");
    assert!(readme.contains("@suspect-fixtures/model-codecs"));
    assert!(readme.contains("models.Flag"));
    assert!(readme.contains("codecs.FlagCodec"));
    assert!(readme.contains("docs-manifest.json"));
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn checked(command: &mut Command) -> Output {
    let output = command.output().expect("native package toolchain required");
    assert!(
        output.status.success(),
        "{command:?}\n{}",
        output_text(&output)
    );
    output
}

struct Toolchain {
    node: PathBuf,
    npm: PathBuf,
    path: OsString,
}

impl Toolchain {
    fn pinned() -> Self {
        let selected = std::env::var_os("SUSPECT_PACKAGE_NODE")
            .or_else(|| std::env::var_os("SUSPECT_DOCS_NODE"))
            .unwrap_or_else(|| "node".into());
        let output = checked(Command::new(selected).args(["--print", "process.execPath"]));
        let node = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
        let bin = node.parent().unwrap();
        let npm = bin
            .parent()
            .unwrap()
            .join("lib/node_modules/npm/bin/npm-cli.js");
        let path = std::env::join_paths(std::iter::once(bin.to_owned()).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let tools = Self { node, npm, path };
        let version = checked(tools.node().arg("--version"));
        assert_eq!(
            String::from_utf8(version.stdout).unwrap().trim(),
            "v22.23.1",
            "select pinned Node through SUSPECT_PACKAGE_NODE"
        );
        let version = checked(tools.npm(Path::new(".")).arg("--version"));
        assert_eq!(String::from_utf8(version.stdout).unwrap().trim(), "10.9.8");
        tools
    }

    fn node(&self) -> Command {
        let mut command = Command::new(&self.node);
        command.env("PATH", &self.path);
        command
    }

    fn npm(&self, root: &Path) -> Command {
        let mut command = self.node();
        command.arg(&self.npm).current_dir(root);
        command
    }
}

fn native(plan: &CodecPlan, javascript: &str, typescript: &str) {
    native_artifacts(
        plan,
        emit(plan, &config()).unwrap(),
        javascript,
        typescript,
        false,
    );
}

fn native_artifacts(
    plan: &CodecPlan,
    files: Vec<OutFile>,
    javascript: &str,
    typescript: &str,
    http: bool,
) {
    let tools = Toolchain::pinned();
    let directory = tempfile::tempdir().unwrap();
    let generation = directory.path().join("generation");
    suspect_codegen::write_files(&files, &generation).unwrap();
    let root = generation.join("typescript");
    checked(tools.npm(&root).args([
        "ci",
        "--offline",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
    ]));
    let version = checked(
        tools
            .node()
            .arg(root.join("node_modules/typescript/bin/tsc"))
            .arg("--version"),
    );
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        "Version 5.9.3"
    );
    checked(tools.npm(&root).args(["run", "build"]));
    let packed =
        checked(
            tools
                .npm(&root)
                .args(["pack", "--offline", "--ignore-scripts", "--json"]),
        );
    let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
    assert_eq!(packed[0]["name"], config().name);
    assert_eq!(packed[0]["version"], config().version);
    let packed_files: BTreeSet<_> = packed[0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    for required in [
        "dist/source/index.js",
        "dist/source/index.d.ts",
        "dist/models.d.ts",
        "dist/model-codecs.js",
        "source/index.ts",
        "README.md",
        "models.md",
        "codecs.md",
        "validation.md",
        "docs-manifest.json",
    ] {
        assert!(
            packed_files.contains(required),
            "tarball is missing {required}"
        );
    }
    assert!(
        !packed_files
            .iter()
            .any(|path| path.starts_with("node_modules/") || path.ends_with(".tgz"))
    );
    let tarball = root.join(packed[0]["filename"].as_str().unwrap());
    let common = r#"
import assert from 'node:assert/strict';
import { readFileSync, existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { codecs, models, JsonNumber, ModelCodecError } from '@suspect-fixtures/model-codecs';
import * as directCodecs from '@suspect-fixtures/model-codecs/codecs';
import { JsonNumber as ModelJsonNumber } from '@suspect-fixtures/model-codecs/models';
import { JsonNumber as DirectJsonNumber, parseJson, stringifyJson } from '@suspect-fixtures/model-codecs/json';
assert.equal(JsonNumber, DirectJsonNumber);
assert.equal(JsonNumber, ModelJsonNumber);
assert.equal(JsonNumber, models.JsonNumber);
assert.equal(ModelCodecError, directCodecs.ModelCodecError);
assert.equal(stringifyJson(parseJson('9007199254740993')), '9007199254740993');
assert.equal(JsonNumber.parse('1.0000000000000000001').toString(), '1.0000000000000000001');
await assert.rejects(import('@suspect-fixtures/model-codecs/dist/codecs.js'), { code: 'ERR_PACKAGE_PATH_NOT_EXPORTED' });
const installed = path.dirname(fileURLToPath(import.meta.resolve('@suspect-fixtures/model-codecs/package.json')));
const metadata = JSON.parse(readFileSync(path.join(installed, 'package.json'), 'utf8'));
assert.equal(metadata.private, true);
assert.equal(metadata.suspect.releaseReady, false);
assert.equal(metadata.suspect.httpClient, expectedHttp);
assert.equal(metadata.dependencies, undefined);
assert.equal(metadata.sideEffects, undefined);
for (const [key, target] of Object.entries(metadata.exports)) {
    if (key === './package.json') continue;
    assert.deepEqual(Object.keys(target), ['types', 'import']);
    assert.ok(existsSync(path.join(installed, target.types)));
    assert.ok(existsSync(path.join(installed, target.import)));
}
function maps(directory) {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
        const file = path.join(directory, entry.name);
        if (entry.isDirectory()) { maps(file); continue; }
        if (!file.endsWith('.map')) continue;
        const map = JSON.parse(readFileSync(file, 'utf8'));
        for (const source of map.sources) {
            const target = path.resolve(path.dirname(file), map.sourceRoot ?? '', source);
            assert.ok(target.startsWith(installed + path.sep));
            assert.ok(existsSync(target), `missing map source: ${source}`);
        }
    }
}
maps(path.join(installed, 'dist'));
"#;
    for language in ["javascript", "typescript"] {
        let consumer = directory.path().join(language);
        std::fs::create_dir(&consumer).unwrap();
        std::fs::write(
            consumer.join("package.json"),
            "{\"private\":true,\"type\":\"module\"}\n",
        )
        .unwrap();
        checked(
            tools
                .npm(&consumer)
                .args([
                    "install",
                    "--offline",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                    "--omit=dev",
                ])
                .arg(&tarball),
        );
        let installed = consumer.join("node_modules/@suspect-fixtures/model-codecs");
        for file in &files {
            let relative = file.path.strip_prefix("typescript/").unwrap();
            if relative == "package-lock.json" {
                continue;
            } // npm intentionally omits it.
            assert_eq!(
                std::fs::read_to_string(installed.join(relative)).unwrap(),
                file.content,
                "installed provenance: {relative}"
            );
        }
        assert!(!installed.join("node_modules").exists());
        std::fs::write(
            consumer.join("consumer.mjs"),
            format!("const expectedHttp = {http};\n{common}\n{javascript}"),
        )
        .unwrap();
        checked(tools.node().current_dir(&consumer).arg("consumer.mjs"));
        if language == "typescript" {
            let mut source = String::from(
                "import { codecs, type models, type ModelCodec } from '@suspect-fixtures/model-codecs';\nimport * as directCodecs from '@suspect-fixtures/model-codecs/codecs';\nimport type * as DirectModels from '@suspect-fixtures/model-codecs/models';\nimport type { Codec } from '@suspect-fixtures/model-codecs/codecs';\n",
            );
            for (index, model) in plan.models().symbols().iter().enumerate() {
                source.push_str(&format!("const root{index}: ModelCodec<models.{0}> = codecs.{0}Codec;\nconst direct{index}: Codec<DirectModels.{0}> = directCodecs.{0}Codec;\n", model.name()));
            }
            source.push_str(typescript);
            std::fs::write(consumer.join("consumer.ts"), source).unwrap();
            let example = content(&files, "typescript/README.md")
                .split_once("```ts\n")
                .unwrap()
                .1
                .split_once("\n```")
                .unwrap()
                .0;
            std::fs::write(consumer.join("readme-example.ts"), example).unwrap();
            checked(
                tools
                    .node()
                    .arg(root.join("node_modules/typescript/bin/tsc"))
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
                        "--outDir",
                        "out",
                        "--pretty",
                        "false",
                        "consumer.ts",
                        "readme-example.ts",
                    ]),
            );
            checked(tools.node().current_dir(&consumer).arg("out/consumer.js"));
            checked(
                tools
                    .node()
                    .current_dir(&consumer)
                    .arg("out/readme-example.js"),
            );
        }
    }
}

#[test]
#[ignore = "requires pinned Node 22.23.1/npm 10.9.8 and cached TypeScript 5.9.3"]
fn installed_openrouter_http_package_runs_a_recorded_request_and_its_documented_helper() {
    let source = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&source).join("projects/docs/openapi/openapi.yaml"));
    let selected = contract
        .operations()
        .filter(|operation| {
            matches!(
                operation.operation_id(),
                Some("getCredits" | "createKeys" | "updateKeys")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    let files = emit_http(&plan, &config()).unwrap();
    for original in plan.render() {
        assert_eq!(content(&files, &original.path), original.content);
    }
    assert_eq!(files, emit_http(&plan, &config()).unwrap());
    native_artifacts(
        plan.codecs(),
        files,
        r#"
import { createServer } from 'node:http';
import { operations, createClient } from '@suspect-fixtures/model-codecs';
import * as directOperations from '@suspect-fixtures/model-codecs/operations';
assert.equal(operations.getCredits, directOperations.getCredits);
assert.equal(metadata.suspect.kind, 'http-client');
assert.ok(existsSync(path.join(installed, 'http-manifest.json')));
const observed = [];
const server = createServer(async (request, response) => {
  let body = '';
  for await (const chunk of request) body += chunk;
  observed.push({method:request.method,url:request.url,auth:request.headers.authorization,accept:request.headers.accept,body});
  response.writeHead(200, {'content-type':'application/json; charset=utf-8'});
  response.end('{"data":{"total_credits":100.0000000000000000001,"total_usage":0.0000000000000000001}}');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
try {
  const serverURL = `http://127.0.0.1:${server.address().port}/api/v1`;
  const client = createClient({auth:{apiKey:'fixture-key'}, serverURL});
  const result = await client.getCredits({});
  assert.equal(result.status, 200);
  assert.equal(result.contentType, 'application/json');
  assert.equal(result.data.data.total_credits.toString(), '100.0000000000000000001');
  assert.equal(result.data.data.total_usage.toString(), '0.0000000000000000001');
  assert.deepEqual(observed, [{method:'GET',url:'/api/v1/credits',auth:'Bearer fixture-key',accept:'application/json',body:''}]);
} finally { await new Promise(resolve => server.close(resolve)); }
"#,
        r#"
import { operations, createClient } from '@suspect-fixtures/model-codecs';
import { callOperation } from './readme-example.js';
const options: operations.ClientOptions = {
  auth: {apiKey:'fixture-key'},
  fetch: async (input, init) => {
    if (String(input) !== 'https://openrouter.ai/api/v1/credits' || init?.method !== 'GET') throw new Error('incorrect documented request');
    return new Response('{"data":{"total_credits":1.0000000000000000001,"total_usage":0}}', {status:200,headers:{'content-type':'application/json'}});
  },
};
const result: operations.GetCreditsSuccess = await callOperation(options, {});
const precise: string = result.data.data.total_credits.toString();
if (precise !== '1.0000000000000000001') throw new Error('documented helper rounded the response');
if (false) {
  // @ts-expect-error: POST body is required
  void createClient(options).createKeys({});
  // @ts-expect-error: create key name is required
  void operations.createKeys(options, {body:{}});
  // @ts-expect-error: null is different from an absent request body
  void operations.createKeys(options, {body:null});
  // @ts-expect-error: response decimals do not become JavaScript numbers
  const rounded: number = result.data.data.total_credits;
}
"#,
        true,
    );
}

#[test]
#[ignore = "requires tracked OpenRouter YAML, pinned Node/npm and cached TypeScript"]
fn installed_openrouter_container_list_and_get_preserve_query_wire_contract() {
    let source = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&source).join("projects/docs/openapi/openapi.yaml"));
    let selected = contract
        .operations()
        .filter(|op| {
            matches!(
                op.operation_id(),
                Some("listContainerFiles" | "getContainerFile")
            )
        })
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    native_artifacts(
        plan.codecs(),
        emit_http(&plan, &config()).unwrap(),
        r#"
import { createClient, operations } from '@suspect-fixtures/model-codecs';
const item = '{"id":"file","object":"container.file","container_id":"sess","bytes":9007199254740993,"created_at":1755640000,"path":"out/report.csv","source":"assistant"}';
const page = '{"object":"list","data":['+item+'],"first_id":"file","last_id":"file","has_more":true}';
const expected = [
  'https://openrouter.ai/api/v1/containers/sess%2F%C3%BC/files?limit=2&after=a%2Fb%20%3F%25%23%2B%20%C3%BC%21%27%28%29%2A%26x%3D1',
  'https://openrouter.ai/api/v1/containers/sess/files',
  'https://openrouter.ai/api/v1/containers/sess/files?after=',
  'https://openrouter.ai/api/v1/containers/sess/files/file%2F%3F%23',
  'https://openrouter.ai/api/v1/containers/sess/files?limit=1',
];
let calls = 0;
const client = createClient({auth:{apiKey:'fixture-key'},fetch:async(url, init)=>{
  assert.equal(String(url), expected[calls++]);
  assert.equal(init.method,'GET'); assert.equal(init.redirect,'error'); assert.equal(init.credentials,'omit'); assert.equal(init.body,undefined);
  assert.equal(new Headers(init.headers).get('authorization'),'Bearer fixture-key');
  return new Response(calls === 4 ? item : page,{status:200,headers:{'content-type':'application/json'}});
}});
const result = await client.listContainerFiles({container_id:'sess/ü',limit:2,after:"a/b ?%#+ ü!'()*&x=1"});
assert.equal(result.data.data[0].bytes,9007199254740993n);
assert.equal(calls,1,'has_more must not trigger inferred pagination');
await client.listContainerFiles({container_id:'sess'});
await client.listContainerFiles({container_id:'sess',after:''});
assert.equal((await client.getContainerFile({container_id:'sess',file_id:'file/?#'})).data.bytes,9007199254740993n);
await client.listContainerFiles({container_id:'sess',limit:1n}); // source-valid encoding, despite the narrower native TS input
for (const invalid of [{}, {container_id:'sess',limit:0}, {container_id:'sess',limit:1001}, {container_id:'sess',after:null}, {container_id:'sess',after:['x']}, {container_id:'sess',authorization:'stolen'}, Object.assign(Object.create({after:'inherited'}),{container_id:'sess'})]) {
  await assert.rejects(client.listContainerFiles(invalid), error=>operations.isSdkError(error) && ['request-validation','request-representation'].includes(error.kind));
}
let touched=false;
await assert.rejects(client.listContainerFiles({container_id:'sess',get after(){touched=true;return 'secret'}}), error=>operations.isSdkError(error) && ['request-validation','request-representation'].includes(error.kind));
assert.equal(touched,false); assert.equal(calls,5,'invalid query never reaches transport');
"#,
        r#"
import { createClient, operations } from '@suspect-fixtures/model-codecs';
const options: operations.ClientOptions = {auth:{apiKey:'fixture-key'},fetch:async(url)=>{
  if(String(url)!=='https://openrouter.ai/api/v1/containers/sess/files?limit=2&after=') throw new Error('typed query differs');
  return new Response('{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}',{status:200,headers:{'content-type':'application/json'}});
}};
const input: operations.ListContainerFilesInput = {container_id:'sess',limit:2,after:''};
const result: operations.ListContainerFilesSuccess = await createClient(options).listContainerFiles(input);
if(result.data.has_more !== false) throw new Error('response contract differs');
if(false){
  // @ts-expect-error: path input is required even though queries are optional
  void createClient(options).listContainerFiles({});
  // @ts-expect-error: bounded integer uses a native number
  void createClient(options).listContainerFiles({container_id:'sess',limit:2n});
  // @ts-expect-error: query null has no declared representation
  void createClient(options).listContainerFiles({container_id:'sess',after:null});
  // @ts-expect-error: optional is omitted, not explicitly undefined
  void createClient(options).listContainerFiles({container_id:'sess',after:undefined});
  // @ts-expect-error: undeclared query members are rejected
  void createClient(options).listContainerFiles({container_id:'sess',offset:2});
  // @ts-expect-error: get flow requires both source path parameters
  void createClient(options).getContainerFile({container_id:'sess'});
}
"#,
        true,
    );
}

#[test]
#[ignore = "requires pinned Node/npm and cached TypeScript"]
fn installed_form_queries_preserve_scalar_array_and_exact_number_semantics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("queries.json");
    let parameters = json!([
        {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
        {"name":"id","in":"query","required":true,"schema":{"type":"string"}},
        {"name":"text","in":"query","schema":{"type":"string"}},
        {"name":"tags","in":"query","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"csv","in":"query","explode":false,"schema":{"type":"array","items":{"type":"string"}}},
        {"name":"flag","in":"query","schema":{"type":"boolean"}},
        {"name":"exact","in":"query","schema":{"type":"number"}},
        {"name":"integer","in":"query","schema":{"type":"integer"}},
        {"name":"numbers","in":"query","schema":{"type":"array","items":{"type":"number"}}},
        {"name":"flags","in":"query","explode":false,"schema":{"type":"array","items":{"type":"boolean"}}}
    ]);
    let response = json!({"200":{"description":"ok","content":{"application/json":{"schema":{"type":"boolean"}}}}});
    let document = json!({"openapi":"3.1.0","info":{"title":"Form vectors","version":"1"},"servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],"components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},"paths":{
        "/items/{id}":{"get":{"operationId":"queryItems","parameters":parameters,"responses":response}},
        "/prototype":{"get":{"operationId":"prototypeQueries","parameters":[
            {"name":"toString","in":"query","schema":{"type":"string"}},
            {"name":"valueOf","in":"query","schema":{"type":"string"}},
            {"name":"constructor","in":"query","schema":{"type":"string"}}
        ],"responses":response}},
        "/required":{"get":{"operationId":"requiredQueries","parameters":[
            {"name":"repeated","in":"query","required":true,"schema":{"type":"array","items":{"type":"string"}}},
            {"name":"compact","in":"query","required":true,"explode":false,"schema":{"type":"array","items":{"type":"string"}}}
        ],"responses":response}}
    }});
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    native_artifacts(
        plan.codecs(),
        emit_http(&plan, &config()).unwrap(),
        r#"
import { createClient, operations } from '@suspect-fixtures/model-codecs';
const expected = [
  'https://example.test/api/v1/items/p%2F%C3%BC?id=q%26injected%3D1&text=&tags=&tags=a%2Cb&tags=%C3%A9%20%21%27%28%29%2A&tags=a%2Cb&csv=a%2Cb,,%26%3D%3F%23%2B&flag=false&exact=1.0000000000000000001e%2B2&integer=9007199254740993&numbers=-0&numbers=1e-100&flags=true,false',
  'https://example.test/api/v1/items/p?id=',
  'https://example.test/api/v1/items/p?id=q&flag=true',
  'https://example.test/api/v1/prototype',
  'https://example.test/api/v1/prototype?toString=a&valueOf=b&constructor=c',
  'https://example.test/api/v1/required?repeated=&compact=',
];
let calls=0;
const client=createClient({auth:{apiKey:'fixture-key'},fetch:async(url,init)=>{
  assert.equal(String(url),expected[calls++]); assert.equal(init.redirect,'error');
  return new Response('true',{status:200,headers:{'content-type':'application/json'}});
}});
await client.queryItems({id:'p/ü',query_id:'q&injected=1',text:'',tags:['','a,b',"é !'()*",'a,b'],csv:['a,b','','&=?#+'],flag:false,exact:JsonNumber.parse('1.0000000000000000001e+2'),integer:9007199254740993n,numbers:[JsonNumber.parse('-0'),JsonNumber.parse('1e-100')],flags:[true,false]});
await client.queryItems({id:'p',query_id:'',tags:[],csv:[],numbers:[],flags:[]});
await client.queryItems({id:'p',query_id:'q',flag:true});
await client.prototypeQueries({});
await client.prototypeQueries({query_toString:'a',query_valueOf:'b',query_constructor:'c'});
await client.requiredQueries({repeated:[''],compact:['']});
for(const input of [{repeated:[],compact:['']},{repeated:[''],compact:[]}]) {
  await assert.rejects(client.requiredQueries(input), error=>operations.isSdkError(error) && error.kind === 'request-representation' && error.source.pointer.includes('/parameters/'));
}
for(const bad of [{id:'p'}, {id:'p',query_id:null}, {id:'p',query_id:'q',tags:[undefined]}, {id:'p',query_id:'q',tags:new Array(1)}, {id:'p',query_id:'q',flag:'false'}, {id:'p',query_id:'q',exact:Object.create(JsonNumber.prototype)}, {id:'p',query_id:'q',exact:{toString(){return '1&credential=secret'}}}, {id:'p',query_id:'q',text:'\ud800'}, {id:'p',query_id:'q',extra:'injection'}]) {
  await assert.rejects(client.queryItems(bad), error=>operations.isSdkError(error) && ['request-validation','request-representation'].includes(error.kind));
}
assert.equal(calls,6,'rejected queries never reach transport');
"#,
        r#"
import { createClient, operations, JsonNumber } from '@suspect-fixtures/model-codecs';
const options: operations.ClientOptions = {auth:{apiKey:'fixture-key'},fetch:async()=>new Response('true',{status:200,headers:{'content-type':'application/json'}})};
const input: operations.QueryItemsInput = {id:'p',query_id:'q',tags:[''],csv:['a,b'],flag:false,exact:JsonNumber.parse('1.0000000000000000001'),integer:9007199254740993n};
if((await createClient(options).queryItems(input)).data !== true) throw new Error('query failed');
// Ordinary objects inherit Object.prototype functions; source query names must
// not make these normal call sites unassignable or send inherited values.
await createClient(options).prototypeQueries({});
await createClient(options).prototypeQueries({query_toString:'a',query_valueOf:'b',query_constructor:'c'});
await createClient(options).requiredQueries({repeated:[''],compact:['']});
if(false){
  // @ts-expect-error: query id is required independently of path id
  void createClient(options).queryItems({id:'p'});
  // @ts-expect-error: strings are not boolean query values
  void createClient(options).queryItems({id:'p',query_id:'q',flag:'false'});
  // @ts-expect-error: decimals require exact representation
  void createClient(options).queryItems({id:'p',query_id:'q',exact:1.1});
  // @ts-expect-error: unbounded integers use bigint
  void createClient(options).queryItems({id:'p',query_id:'q',integer:1});
  // @ts-expect-error: array items are non-null source strings
  void createClient(options).queryItems({id:'p',query_id:'q',tags:[null]});
  // @ts-expect-error: required query arrays must be present
  void createClient(options).requiredQueries({});
}
"#,
        true,
    );
}

#[test]
#[ignore = "requires pinned Node 22.23.1/npm 10.9.8 and cached TypeScript 5.9.3"]
fn installed_package_preserves_colliding_models_and_support_apis() {
    let plan = plan(json!({
        "Model":{"type":"integer"}, "ModelCodec":{"type":"boolean"},
        "ModelCodecError":{"type":"string"}, "JsonLimits":{"type":"string"},
        "ValidationSource":{"type":"string"}, "ValidationFinding":{"type":"boolean"},
        "Codec":{"type":"boolean"},
        "Presence":{"type":"object","additionalProperties":false,"required":["nullable"],"properties":{
            "nullable":{"type":["string","null"]},"optional":{"type":"string"}
        }}
    }));
    native(
        &plan,
        r#"
assert.equal(codecs.ModelCodec.decode('9007199254740993'), 9007199254740993n);
assert.equal(codecs.ModelCodec, directCodecs.ModelCodec);
assert.equal(codecs.ModelCodecCodec.decode('true'), true);
assert.equal(codecs.ModelCodecErrorCodec.decode('"source"'), 'source');
assert.equal(codecs.JsonLimitsCodec.decode('"limits"'), 'limits');
assert.equal(codecs.ValidationSourceCodec.decode('"origin"'), 'origin');
assert.equal(codecs.ValidationFindingCodec.decode('false'), false);
assert.equal(codecs.CodecCodec.decode('true'), true);
const presence = codecs.PresenceCodec.decode('{"nullable":null}');
assert.equal(presence.nullable, null);
assert.equal(Object.hasOwn(presence, 'optional'), false);
assert.equal(codecs.PresenceCodec.encode({ ...presence, optional: undefined }), '{"nullable":null}');
assert.throws(() => codecs.PresenceCodec.decode('{}'), e => e instanceof ModelCodecError && e.kind === 'invalid');
"#,
        r#"
import type { JsonLimits, ValidationSource, ValidationFinding } from '@suspect-fixtures/model-codecs';
const limits: JsonLimits = { maxNodes: 100 };
const source: ValidationSource = { document: 'api.json', pointer: '/components/schemas/Model' };
const finding: ValidationFinding = { source, instancePath: '', message: 'test' };
const integer: models.Model = codecs.ModelCodec.decode('9007199254740993');
const nullable: models.Presence = { nullable: null };
// @ts-expect-error exact integer model rejects lossy JavaScript numbers
const rounded: models.Model = 9007199254740993;
// @ts-expect-error required nullable property cannot be omitted
const missing: models.Presence = {};
// @ts-expect-error exact optional properties omit undefined
const undefinedOptional: models.Presence = { nullable: null, optional: undefined };
if (integer !== 9007199254740993n) throw new Error('incorrect installed declaration consumer');
"#,
    );
}

#[test]
#[ignore = "requires tracked OpenRouter YAML, pinned Node/npm and cached TypeScript"]
fn installed_package_preserves_tracked_openrouter_caller_and_image_closures() {
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&checkout).join("projects/docs/openapi/openapi.yaml"));
    let roots = contract
        .schema_roots()
        .iter()
        .filter(|id| {
            [
                "/components/schemas/ORAnthropicNullableCaller",
                "/components/schemas/AnthropicImageBlockParam",
            ]
            .contains(&id.pointer())
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    native(
        &plan,
        r#"
assert.equal(codecs.ORAnthropicNullableCallerCodec.decode('null'), null);
assert.equal(codecs.ORAnthropicNullableCallerCodec.decode('{"type":"direct"}').type, 'direct');
const caller = '{"type":"code_execution_20260120","tool_id":"tool"}';
assert.equal(codecs.ORAnthropicNullableCallerCodec.encode(codecs.ORAnthropicNullableCallerCodec.decode(caller)), caller);
const image = '{"type":"image","source":{"type":"url","url":"https://example.test/image"}}';
assert.equal(codecs.AnthropicImageBlockParamCodec.encode(codecs.AnthropicImageBlockParamCodec.decode(image)), image);
const base64 = '{"type":"image","source":{"type":"base64","media_type":"image/png","data":"abc"}}';
assert.equal(codecs.AnthropicImageBlockParamCodec.encode(codecs.AnthropicImageBlockParamCodec.decode(base64)), base64);
for (const [codec, text] of [
    [codecs.ORAnthropicNullableCallerCodec, '{"type":"code_execution_20260120"}'],
    [codecs.AnthropicImageBlockParamCodec, '{"type":"image","source":{"type":"base64","url":"https://example.test/image"}}'],
]) assert.throws(() => codec.decode(text), e => e instanceof ModelCodecError && e.kind === 'invalid');
"#,
        r#"
const caller: models.ORAnthropicNullableCaller = { type: 'code_execution_20260120', tool_id: 'tool' };
const image: models.AnthropicImageBlockParam = { type: 'image', source: { type: 'url', url: 'https://example.test/image' } };
// @ts-expect-error the selected caller variant requires its tool identifier
const missingTool: models.ORAnthropicNullableCaller = { type: 'code_execution_20260120' };
// @ts-expect-error base64 variant requires data and media_type
const missingImage: models.AnthropicImageBlockParam = { type: 'image', source: { type: 'base64', url: 'https://example.test/image' } };
if (codecs.ORAnthropicNullableCallerCodec.decode('null') !== null) throw new Error('caller null changed');
if (codecs.AnthropicImageBlockParamCodec.decode(codecs.AnthropicImageBlockParamCodec.encode(image)).type !== 'image') throw new Error('image changed');
"#,
    );
}
