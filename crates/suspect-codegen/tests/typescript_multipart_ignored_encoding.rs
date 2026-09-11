//! Literal MIME witnesses for ignored multipart/mixed Encoding Object styles.

#![cfg(feature = "http-protocol")]

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    http_protocol as wire,
    typescript::{
        http::{HttpConfig, plan_http},
        package::{PackageConfig, emit_http},
    },
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().expect("required native TypeScript tool");
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn directory() -> (PathBuf, Option<tempfile::TempDir>) {
    if let Some(parent) = std::env::var_os("SUSPECT_PROTOCOL_ARTIFACTS") {
        std::fs::create_dir_all(&parent).unwrap();
        let root = tempfile::Builder::new()
            .prefix("ignored-multipart-")
            .tempdir_in(parent)
            .unwrap()
            .keep();
        println!("retained ignored multipart evidence: {}", root.display());
        (root, None)
    } else {
        let temporary = tempfile::tempdir().unwrap();
        (temporary.path().to_owned(), Some(temporary))
    }
}

#[test]
#[ignore = "requires pinned Node 22/24 and offline npm/TypeScript 5.5/5.9"]
fn ignored_multipart_styles_preserve_content_in_installed_requests_and_responses() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let witness = &fixture["ignoredMultipartEncoding"];
    let (root, _temporary) = directory();
    let entry = root.join("api.json");
    std::fs::write(
        &entry,
        serde_json::to_vec_pretty(&witness["document"]).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("source-witness.json"),
        serde_json::to_vec_pretty(witness).unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::expanded()).unwrap();
    assert_eq!(plan.operations().len(), 1);
    assert_eq!(
        plan.protocol()
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        witness["codecRoots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    let operation = &plan.operations()[0];
    let protocol = operation.protocol();
    for media in [
        &protocol.body().unwrap().media()[0],
        &protocol.responses()[0].media()[0],
    ] {
        assert_eq!(
            media.source().terminal().source().pointer(),
            "/components/mediaTypes/Mixed"
        );
        let wire::Representation::Multipart {
            multipart: wire::MultipartPlan::Positional { prefix, items, .. },
        } = media.representation()
        else {
            panic!("source-selected positional multipart/mixed")
        };
        assert_eq!(prefix.len(), 4);
        for (part, expected) in prefix
            .iter()
            .zip(witness["prefixMedia"].as_array().unwrap())
        {
            assert_eq!(
                part.content_types()
                    .iter()
                    .map(|media| media.declared())
                    .collect::<Vec<_>>(),
                [expected.as_str().unwrap()]
            );
            assert_eq!(part.multiplicity(), wire::PartMultiplicity::One);
            assert!(!matches!(
                part.representation(),
                wire::PartRepresentation::Style { .. }
            ));
        }
        assert!(matches!(
            prefix[0].representation(),
            wire::PartRepresentation::Json { .. }
        ));
        assert!(prefix[0].headers()[0].required());
        assert!(matches!(
            prefix[1].representation(),
            wire::PartRepresentation::Text {
                scalar: wire::ScalarType::Integer,
                ..
            }
        ));
        let wire::PartRepresentation::Json { codec, .. } = prefix[2].representation() else {
            panic!("the array is one JSON part")
        };
        assert_eq!(
            codec.schema().id().pointer(),
            "/components/mediaTypes/Mixed/schema/prefixItems/2"
        );
        assert!(matches!(
            prefix[3].representation(),
            wire::PartRepresentation::Binary { .. }
        ));
        let wire::AdditionalParts::Allowed(item) = items else {
            panic!("source itemEncoding")
        };
        assert_eq!(
            item.content_types()[0].declared(),
            witness["itemMedia"].as_str().unwrap()
        );
        assert!(matches!(
            item.representation(),
            wire::PartRepresentation::Json { .. }
        ));
    }
    let warnings = plan.protocol().diagnostics();
    for suffix in [
        "/prefixEncoding/0/style",
        "/prefixEncoding/0/explode",
        "/prefixEncoding/1/style",
        "/prefixEncoding/2/explode",
        "/prefixEncoding/3/allowReserved",
        "/itemEncoding/style",
        "/itemEncoding/explode",
        "/itemEncoding/allowReserved",
    ] {
        assert!(
            warnings
                .iter()
                .any(|finding| finding.code() == "http-encoding-style-ignored"
                    && finding.source().source().pointer().ends_with(suffix)
                    && !finding.source().span().is_empty()),
            "{suffix}: {warnings:?}"
        );
    }
    assert!(
        !warnings
            .iter()
            .any(|finding| finding.code() == "http-encoding-content-type-ignored")
    );
    std::fs::write(
        root.join("protocol.json"),
        serde_json::to_vec_pretty(plan.protocol()).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("native-interface.json"),
        serde_json::to_vec_pretty(operation.interface()).unwrap(),
    )
    .unwrap();
    let types = TYPES.replace("__OP__", &operation.function_name);
    let consumer = CONSUMER.replace("__OP__", &operation.function_name);
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: "@fixture/ignored-multipart".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let package = root.join("typescript");
    let selected_node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let executable = checked(
        Command::new(&selected_node).args(["--print", "process.execPath"]),
        &root,
    );
    let node = PathBuf::from(String::from_utf8(executable.stdout).unwrap().trim());
    let node24 = std::env::var_os("SUSPECT_NODE24_BIN")
        .expect("set SUSPECT_NODE24_BIN for the second native runtime");
    for (executable, major) in [(node.as_os_str(), "22"), (node24.as_os_str(), "24")] {
        let version = checked(
            Command::new(executable).args(["--print", "process.versions.node.split('.')[0]"]),
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
    let floor = root.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let reviewed = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-floor");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(reviewed.join(file), floor.join(file)).unwrap();
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
        let packed = checked(
            npm_command(&package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
            &root,
        );
        let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
        let archive = root.join(format!("ignored-multipart-ts{version}.tgz"));
        std::fs::rename(
            package.join(packed[0]["filename"].as_str().unwrap()),
            &archive,
        )
        .unwrap();
        let installed = root.join(format!("installed-{version}"));
        std::fs::create_dir(&installed).unwrap();
        std::fs::write(
            installed.join("package.json"),
            "{\"private\":true,\"type\":\"module\"}",
        )
        .unwrap();
        checked(
            npm_command(&installed)
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
        std::fs::write(installed.join("consumer.ts"), &types).unwrap();
        std::fs::write(installed.join("consumer.mjs"), &consumer).unwrap();
        checked(
            Command::new(&node)
                .arg(&compiler)
                .current_dir(&installed)
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
                    .current_dir(&installed)
                    .arg("node_modules/@fixture/ignored-multipart/dist/examples/validated.js"),
                &root,
            );
            let result = checked(
                Command::new(executable)
                    .current_dir(&installed)
                    .arg("consumer.mjs"),
                &root,
            );
            println!(
                "TypeScript {version}: {}",
                String::from_utf8_lossy(&result.stdout).trim()
            );
        }
    }
}

const TYPES: &str = r#"
import {createClient} from '@fixture/ignored-multipart';
const client = createClient();
export async function mixedContent() {
    const reply = await client.__OP__({body:[
        {data:'snow 雪?x=1&y=%2F',headers:{'X-Part':9007199254740993n},contentType:'application/json'},
        9007199254740995n, ['a,b','snow 雪','%2F'], new Uint8Array([0,255,128]), {n:9007199254740997n},
    ]});
    const text:string = reply.data[0].data;
    const header:bigint = reply.data[0].headers['X-Part'];
    const integer:bigint = reply.data[1];
    const array:readonly string[] = reply.data[2];
    const bytes:Uint8Array = reply.data[3].data;
    const tail:bigint|undefined = reply.data[4]?.n;
    await client.__OP__({body:[{data:'',headers:{'X-Part':1n}},2n,[],new Uint8Array()]});
    // @ts-expect-error ignored styles do not remove the required part header
    await client.__OP__({body:[{data:'x'},2n,[],new Uint8Array()]});
    // @ts-expect-error explicit JSON string content still needs a string model
    await client.__OP__({body:[{data:1n,headers:{'X-Part':1n}},2n,[],new Uint8Array()]});
    // @ts-expect-error the default text integer retains bigint representation
    await client.__OP__({body:[{data:'x',headers:{'X-Part':1n}},'2',[],new Uint8Array()]});
    // @ts-expect-error ignored explode does not turn the whole array into text
    await client.__OP__({body:[{data:'x',headers:{'X-Part':1n}},2n,'a,b',new Uint8Array()]});
    // @ts-expect-error raw bytes are neither a string nor a placeholder JSON value
    await client.__OP__({body:[{data:'x',headers:{'X-Part':1n}},2n,[],'bytes']});
    // @ts-expect-error itemEncoding retains the native JSON object's integer field
    await client.__OP__({body:[{data:'x',headers:{'X-Part':1n}},2n,[],new Uint8Array(),{n:'3'}]});
    // @ts-expect-error all four prefix positions are required by minItems
    await client.__OP__({body:[{data:'x',headers:{'X-Part':1n}},2n,[]]});
    return {text,header,integer,array,bytes,tail};
}
"#;

const CONSUMER: &str = r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {mkdirSync,writeFileSync} from 'node:fs';
import {createClient,operations} from '@fixture/ignored-multipart';

const evidence=`wire-${process.versions.node}`;mkdirSync(evidence);
const binary=Buffer.from([0,255,128,13,10,37,50,70]);
const replyBinary=Buffer.from([254,0,255,13,10,37,50,70]);
const expectedRequest=boundary=>Buffer.concat([
    Buffer.from(`--${boundary}\r\nX-Part: 9007199254740993\r\nContent-Type: application/json\r\n\r\n"snow 雪?x=1&y=%2F"\r\n--${boundary}\r\nContent-Type: text/plain\r\n\r\n9007199254740995\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n["a,b","snow 雪","%2F"]\r\n--${boundary}\r\nContent-Type: application/octet-stream\r\n\r\n`),
    binary,
    Buffer.from(`\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n{"n":9007199254740997}\r\n--${boundary}--\r\n`),
]);
const responseParts=[
    Buffer.from('Content-Type: application/json\r\nX-Part: 9007199254740999\r\n\r\n"reply = snow 雪%2F"'),
    Buffer.from('\r\n9007199254741001'), // MIME defaults to text/plain when headers are absent.
    Buffer.from('Content-Type: application/json\r\n\r\n["reply,a","snow 雪","[x]=y&z"]'),
    Buffer.concat([Buffer.from('Content-Type: application/octet-stream\r\n\r\n'),replyBinary]),
    Buffer.from('Content-Type: application/json\r\n\r\n{"n":9007199254741003}'),
];
function responseBytes(mode){
    const parts=responseParts.map(part=>Buffer.from(part));
    if(mode==='missing-header')parts[0]=Buffer.from('Content-Type: application/json\r\n\r\n"reply"');
    if(mode==='wrong-header')parts[0]=Buffer.from('Content-Type: application/json\r\nX-Part: wrong\r\n\r\n"reply"');
    if(mode==='raw-string')parts[0]=Buffer.from('Content-Type: application/json\r\nX-Part: 1\r\n\r\nreply');
    if(mode==='wrong-media')parts[0]=Buffer.from('Content-Type: text/plain\r\nX-Part: 1\r\n\r\n"reply"');
    if(mode==='style-array')parts[2]=Buffer.from('Content-Type: application/json\r\n\r\na,b,c');
    if(mode==='binary-media')parts[3]=Buffer.concat([Buffer.from('Content-Type: text/plain\r\n\r\n'),replyBinary]);
    if(mode==='bad-tail')parts[4]=Buffer.from('Content-Type: application/json\r\n\r\n{"n":"wrong"}');
    if(mode==='too-many')parts.push(Buffer.from('Content-Type: application/json\r\n\r\n{}'));
    if(mode==='too-few')parts.splice(3);
    if(mode==='empty-array-no-tail'){parts[2]=Buffer.from('Content-Type: application/json\r\n\r\n[]');parts.pop();}
    return Buffer.concat([...parts.flatMap(part=>[Buffer.from('--ignored-response\r\n'),part,Buffer.from('\r\n')]),Buffer.from('--ignored-response--\r\n')]);
}
const requests=[];let mode='valid',serverError;
const server=createServer(async(request,response)=>{
    try{
        assert.equal(request.method,'POST');assert.equal(request.url,'/parts');
        const header=request.headers['content-type'];const match=/^multipart\/mixed;boundary=([0-9A-Za-z-]+)$/.exec(header);assert.ok(match,header);
        const chunks=[];for await(const chunk of request)chunks.push(chunk);const body=Buffer.concat(chunks);
        assert.deepEqual(body,expectedRequest(match[1]),'request uses five physical content parts, with exact JSON/text/raw-byte payloads');
        requests.push({header,body});const reply=responseBytes(mode);
        if(requests.length===1){writeFileSync(`${evidence}/request.bin`,body);writeFileSync(`${evidence}/request-content-type.txt`,header);writeFileSync(`${evidence}/response.bin`,reply);}
        response.writeHead(200,{'content-type':'multipart/mixed;boundary=ignored-response'});response.end(reply);
    }catch(error){serverError=error;response.writeHead(500,{'content-type':'text/plain'});response.end(String(error.stack));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
    const body=[{data:'snow 雪?x=1&y=%2F',headers:{'X-Part':9007199254740993n},contentType:'application/json'},9007199254740995n,['a,b','snow 雪','%2F'],new Uint8Array(binary),{n:9007199254740997n}];
    const reply=await client.__OP__({body});
    assert.equal(reply.data[0].data,'reply = snow 雪%2F');assert.equal(reply.data[0].headers['X-Part'],9007199254740999n);assert.equal(reply.data[0].contentType,'application/json');
    assert.equal(reply.data[1],9007199254741001n);assert.deepEqual(reply.data[2],['reply,a','snow 雪','[x]=y&z']);
    assert.deepEqual([...reply.data[3].data],[254,0,255,13,10,37,50,70]);assert.equal(reply.data[3].contentType,'application/octet-stream');assert.equal(reply.data[4].n,9007199254741003n);
    assert.ok(!requests[0].body.includes(Buffer.from('Content-Disposition:')),'positional MIME fields never acquire form-data names');
    mode='empty-array-no-tail';const minimal=await client.__OP__({body});assert.equal(minimal.data.length,4);assert.deepEqual(minimal.data[2],[]);
    const before=requests.length;
    const requestControls=[
        [body.slice(0,3),'request-validation'],[[...body,{}],'request-validation'],
        [[{data:'x'},...body.slice(1)],'request-representation'],
        [[{...body[0],data:1n},...body.slice(1)],'request-validation'],
        [[{...body[0],headers:{'X-Part':'wrong'}},...body.slice(1)],'request-validation'],
        [[{...body[0],contentType:'text/plain'},...body.slice(1)],'request-representation'],
        [[body[0],'9007199254740995',...body.slice(2)],'request-validation'],
        [[body[0],body[1],'a,b',...body.slice(3)],'request-validation'],
        [[...body.slice(0,3),'not bytes',body[4]],'request-representation'],
        [[...body.slice(0,4),{n:'wrong'}],'request-validation'],
    ];
    for(const [invalid,kind] of requestControls)await assert.rejects(client.__OP__({body:invalid}),error=>operations.isSdkError(error)&&error.kind===kind);
    await assert.rejects(createClient({serverURL,maxPartBytes:1}).__OP__({body}),error=>operations.isSdkError(error)&&error.kind==='resource-limit');
    assert.equal(requests.length,before,'invalid content, headers and cardinalities are rejected before transport');
    const responseControls=['missing-header','wrong-header','raw-string','wrong-media','style-array','binary-media','bad-tail','too-many','too-few'];
    for(mode of responseControls)await assert.rejects(client.__OP__({body}),error=>operations.isSdkError(error)&&error.kind==='response-decoding');
    assert.equal(serverError,undefined);
    const result={node:process.versions.node,exchanges:requests.length,requestControls:requestControls.length+1,responseControls:responseControls.length,requestParts:5,responseParts:5,minimalResponseParts:4};
    writeFileSync(`${evidence}/result.json`,JSON.stringify(result,null,2)+'\n');
    console.log('ignored-multipart-content-passed',JSON.stringify(result));
}finally{await new Promise(resolve=>server.close(resolve));}
"#;
