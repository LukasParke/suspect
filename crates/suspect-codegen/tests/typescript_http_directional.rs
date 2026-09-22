//! Source-directional HTTP inputs/results through the installed native runtime and docs.

use serde_json::json;
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::typescript::http::{HttpConfig, plan_http};
use suspect_codegen::typescript::package::{PackageConfig, emit_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[test]
#[ignore = "requires pinned native TypeScript, Node 22 and TypeDoc tools"]
fn directional_http_codecs_preserve_supplied_fields_and_validate_both_directions() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.keep();
    let input = directory.join("api.json");
    std::fs::write(&input,json!({
        "openapi":"3.1.0","info":{"title":"Directional HTTP","version":"1"},
        "servers":[{"url":"https://example.test/v1"}],"security":[{"apiKey":[]}],
        "components":{
            "securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
            "schemas":{"Entity":{
                "type":"object","required":["id","secret","label"],
                "properties":{
                    "id":{"type":"integer","minimum":1,"readOnly":true},
                    "secret":{"type":"string","minLength":2,"writeOnly":true},
                    "label":{"type":"string","minLength":2},
                    "note":{"type":["string","null"]}
                },"additionalProperties":false
            }}
        },
        "paths":{"/entity":{"put":{
            "operationId":"replaceEntity","description":"Replace an entity using source direction annotations.",
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Entity"}}}},
            "responses":{"200":{"description":"The entity.","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Entity"}}}}}
        }}}
    }).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&directory).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&input).unwrap()).unwrap());
    let source = contract.operations().next().unwrap().source().clone();
    let plan = plan_http(contract, &[source], HttpConfig::default()).unwrap();
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: "@fixture/directional-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &directory).unwrap();
    let root = directory.join("typescript");
    let npm_global = directory.join("npm-global.config");
    std::fs::write(&npm_global, "").unwrap();
    for args in [
        vec![
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ],
        vec!["run", "build"],
        vec!["pack", "--offline", "--ignore-scripts"],
    ] {
        let output = Command::new("npm")
            .args(args)
            .current_dir(&root)
            .env("NPM_CONFIG_USERCONFIG", "/dev/null")
            .env("NPM_CONFIG_GLOBALCONFIG", &npm_global)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture {}\n{}{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let consumer = directory.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("package.json"),
        "{\"private\":true,\"type\":\"module\"}",
    )
    .unwrap();
    let installed = Command::new("npm")
        .args([
            "install",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ])
        .arg(root.join("fixture-directional-sdk-0.0.0.tgz"))
        .current_dir(&consumer)
        .env("NPM_CONFIG_USERCONFIG", "/dev/null")
        .env("NPM_CONFIG_GLOBALCONFIG", &npm_global)
        .output()
        .unwrap();
    assert!(
        installed.status.success(),
        "fixture {}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&installed.stdout),
        String::from_utf8_lossy(&installed.stderr)
    );
    std::fs::write(
        consumer.join("consumer.ts"),
        r#"
import { operations } from '@fixture/directional-sdk';
const { replaceEntity, createClient } = operations;
export function inputTypes(client: Parameters<typeof replaceEntity>[0]) {
    void replaceEntity(client, { body: { label: 'ok', secret: 'private' } });
    void replaceEntity(client, { body: { label: 'ok', secret: 'private', id: 1n, note: null } });
    // @ts-expect-error A request still requires its write-only property.
    void replaceEntity(client, { body: { label: 'ok' } });
    // @ts-expect-error Ordinary required properties retain requiredness.
    void replaceEntity(client, { body: { secret: 'private' } });
    const bound = createClient(client);
    return bound.replaceEntity({ body: { label: 'ok', secret: 'private' } }).then(response => {
        const id: bigint = response.data.id;
        const secret: string | undefined = response.data.secret;
        return { id, secret };
    });
}
"#,
    )
    .unwrap();
    let output = Command::new("tsc")
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
            "consumer.ts",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture {}\n{}{}",
        directory.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(consumer.join("consumer.mjs"),r#"
import assert from 'node:assert/strict';
import { operations } from '@fixture/directional-sdk';
const { createClient, isSdkError } = operations;
(async () => {
    let calls=0;
    let response='{"id":9007199254740993,"label":"ok","note":null}';
    const bodies=[];
    const client=createClient({auth:{apiKey:'secret'},fetch:async(url,init)=>{
        calls++;
        assert.equal(String(url),'https://example.test/v1/entity');
        assert.equal(init.method,'PUT');
        bodies.push(init.body);
        return new Response(response,{status:200,headers:{'content-type':'application/json'}});
    }});
    const first=await client.replaceEntity({body:{label:'ok',secret:'private'}});
    assert.equal(first.data.id,9007199254740993n);
    assert.equal(Object.hasOwn(first.data,'secret'),false);
    assert.equal(first.data.note,null);
    assert.deepEqual(JSON.parse(bodies[0]),{label:'ok',secret:'private'});
    await client.replaceEntity({body:{id:2n,label:'ok',secret:'private',note:null}});
    assert.deepEqual(JSON.parse(bodies[1]),{id:2,label:'ok',secret:'private',note:null});
    for (const body of [{label:'ok'},{secret:'private'},{id:0n,label:'ok',secret:'private'}]) {
        const before=calls;
        await assert.rejects(client.replaceEntity({body}),error=>isSdkError(error)&&error.kind==='request-validation');
        assert.equal(calls,before);
    }
    for (const body of ['{"label":"ok"}','{"id":1,"label":"ok","secret":"x"}']) {
        response=body;
        await assert.rejects(client.replaceEntity({body:{label:'ok',secret:'private'}}),error=>isSdkError(error)&&error.kind==='response-decoding');
    }
})().catch(error=>{ console.error(error);process.exitCode=1; });
"#).unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    for args in [
        vec![consumer.join("consumer.mjs")],
        vec![
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs"),
            root.clone(),
        ],
    ] {
        let output = Command::new(&node).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "fixture {}\n{}{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Equal erased `never` shapes cannot hide a predicate bound to a different
    // written source alias. This intentionally corrupts only the negative fixture.
    let operation_file = root.join("operations.ts");
    let original = std::fs::read_to_string(&operation_file).unwrap();
    std::fs::write(&operation_file, format!("{}\n/** A deliberately different empty error alias for the negative binding test. */\nexport type OtherEmptyApiError = never;\n", original.replace("error is ReplaceEntityApiError", "error is OtherEmptyApiError"))).unwrap();
    let rejected = Command::new(&node)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs"))
        .arg(&root)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("compiler error binding drifted"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    std::fs::remove_dir_all(directory).unwrap();
}
