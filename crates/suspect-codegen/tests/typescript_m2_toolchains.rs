//! Installed declaration consumers on TypeScript 5.5.4/5.9.3 and real JS on
//! pinned Node 22 plus an explicitly selected Node 24 runtime.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};
use suspect_codegen::typescript::{
    http::{HttpConfig, plan_http},
    package::{PackageConfig, emit_http},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const PACKAGE: &str = "@suspect-fixtures/m2-toolchains";
const OPERATIONS: [&str; 5] = [
    "getCredits",
    "createKeys",
    "updateKeys",
    "listContainerFiles",
    "getContainerFile",
];

fn checked(command: &mut Command, retained: &Path) -> Output {
    let output = command.output().expect("required native tool missing");
    assert!(
        output.status.success(),
        "fixture {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Tools {
    node: PathBuf,
    node24: PathBuf,
    npm: PathBuf,
}
impl Tools {
    fn node(&self) -> Command {
        let mut command = Command::new(&self.node);
        command.env(
            "PATH",
            std::env::join_paths(
                std::iter::once(self.node.parent().unwrap().to_owned()).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ),
            )
            .unwrap(),
        );
        command
    }
    fn npm(&self, cwd: &Path) -> Command {
        let mut command = self.node();
        command.arg(&self.npm).current_dir(cwd);
        command
    }
}

#[test]
#[ignore = "requires pinned Node 22/npm/TS caches, explicit SUSPECT_NODE24_BIN and tracked OpenRouter corpus"]
fn installed_sdk_checks_floor_current_types_and_executes_both_node_runtimes() {
    let temporary = tempfile::tempdir().unwrap();
    let retained = temporary.keep();
    let selected = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let output = checked(
        Command::new(&selected).args(["--print", "process.execPath"]),
        &retained,
    );
    let node = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let node24 = PathBuf::from(
        std::env::var_os("SUSPECT_NODE24_BIN").expect("set SUSPECT_NODE24_BIN to Node 24"),
    );
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let tools = Tools { node, node24, npm };
    let version = checked(tools.node().arg("--version"), &retained);
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        "v22.23.1"
    );
    let version = checked(tools.npm(&retained).arg("--version"), &retained);
    assert_eq!(String::from_utf8(version.stdout).unwrap().trim(), "10.9.8");
    let version = checked(Command::new(&tools.node24).arg("--version"), &retained);
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .trim()
            .starts_with("v24.")
    );

    let source =
        PathBuf::from(std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"))
            .join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let selected = OPERATIONS
        .iter()
        .map(|id| {
            contract
                .operations()
                .find(|op| op.operation_id() == Some(*id))
                .unwrap_or_else(|| panic!("missing source operation {id}"))
                .source()
                .clone()
        })
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: PACKAGE.into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &retained).unwrap();
    let package = retained.join("typescript");
    checked(
        tools.npm(&package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &retained,
    );
    checked(tools.npm(&package).args(["run", "build"]), &retained);
    let packed = checked(
        tools
            .npm(&package)
            .args(["pack", "--offline", "--ignore-scripts", "--json"]),
        &retained,
    );
    let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
    assert_eq!(packed[0]["name"], PACKAGE);
    let tarball = package.join(packed[0]["filename"].as_str().unwrap());

    let floor = retained.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let pinned = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-floor");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(pinned.join(file), floor.join(file)).unwrap();
    }
    checked(
        tools.npm(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &retained,
    );

    let consumer = retained.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        "{\"private\":true,\"type\":\"module\"}",
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
            ])
            .arg(&tarball),
        &retained,
    );
    std::fs::write(
        consumer.join("tsconfig.json"),
        json!({"compilerOptions": {
        "strict":true, "exactOptionalPropertyTypes":true, "noUncheckedIndexedAccess":true,
        "target":"ES2022", "module":"NodeNext", "moduleResolution":"NodeNext", "noEmit":true
    }, "files":["consumer.ts"]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        consumer.join("consumer.ts"),
        r#"
import {createClient, operations, JsonNumber} from '__PACKAGE__';
const options: operations.ClientOptions = {auth:{apiKey:'fixture-only'}};
const client = createClient(options);
export async function nativeInputs() {
    const credits = await client.getCredits({});
    const exact: JsonNumber = credits.data.data.total_credits;
    await client.createKeys({body:{name:'CI'}});
    await client.updateKeys({hash:'key',body:{limit:null}});
    await client.listContainerFiles({container_id:'container'});
    await client.getContainerFile({container_id:'container',file_id:'file'});
    return exact;
}
export function rejectedInputs() {
    // @ts-expect-error The declared security scheme is required.
    createClient({auth:{}});
    // @ts-expect-error A required body field cannot be omitted.
    client.createKeys({body:{}});
    // @ts-expect-error Required string is not a JSON number.
    client.createKeys({body:{name:42}});
    // @ts-expect-error A required path component cannot be omitted.
    client.getContainerFile({container_id:'container'});
    // @ts-expect-error Explicit undefined is not an absent optional property.
    client.createKeys({body:{name:'CI',limit:undefined}});
}
"#
        .replace("__PACKAGE__", PACKAGE),
    )
    .unwrap();
    for (version, root) in [("5.5.4", &floor), ("5.9.3", &package)] {
        let tsc = root.join("node_modules/typescript/bin/tsc");
        let output = checked(tools.node().arg(&tsc).arg("--version"), &retained);
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            format!("Version {version}")
        );
        checked(
            tools
                .node()
                .arg(&tsc)
                .args(["--project", "tsconfig.json"])
                .current_dir(&consumer),
            &retained,
        );
    }

    std::fs::write(consumer.join("consumer.mjs"), r#"
import assert from 'node:assert/strict';
import {createClient, operations, parseJson, stringifyJson, JsonNumber, JsonCodecError} from '__PACKAGE__';
let calls = 0;
const client = createClient({auth:{apiKey:'fixture-only'}, fetch:async (url, init) => {
    calls++;
    assert.equal(String(url),'https://openrouter.ai/api/v1/credits');
    assert.equal(init.method,'GET');
    assert.equal(new Headers(init.headers).get('authorization'),'Bearer fixture-only');
    assert.equal(init.redirect,'error');
    return new Response('{"data":{"total_credits":9007199254740993.25,"total_usage":1e-400}}',{status:200,headers:{'Content-Type':'application/json'}});
}});
for(const id of ['getCredits','createKeys','updateKeys','listContainerFiles','getContainerFile']) {
    assert.equal(typeof client[id],'function'); assert.equal(typeof operations[id],'function');
}
const response = await client.getCredits({});
assert.equal(response.status,200);
assert.equal(response.data.data.total_credits.toString(),'9007199254740993.25');
assert.equal(response.data.data.total_usage.toString(),'1e-400');
assert.equal(calls,1);
const parsed = parseJson('{"n":100000000000000000000,"f":1.5,"e":1e3}');
assert.equal(stringifyJson(parsed),'{"n":100000000000000000000,"f":1.5,"e":1e3}');
assert.ok(parsed.n instanceof JsonNumber);
assert.throws(()=>parseJson('{"k":1,"k":2}'),error=>error instanceof JsonCodecError && error.kind==='duplicate-key');
console.log('installed-consumer-ok',process.version);
"#.replace("__PACKAGE__", PACKAGE)).unwrap();
    for runtime in [&tools.node, &tools.node24] {
        let output = checked(
            Command::new(runtime)
                .arg("consumer.mjs")
                .current_dir(&consumer),
            &retained,
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("installed-consumer-ok"));
        println!("{}", text.trim());
    }
    std::fs::remove_dir_all(retained).unwrap();
}
