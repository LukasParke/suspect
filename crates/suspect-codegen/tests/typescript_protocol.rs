//! Native witnesses at Contract -> TypeScript HTTP package -> installed Fetch
//! consumer. Wire expectations are literal normative examples, not emitter snapshots.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::typescript::{
    http::{HttpConfig, HttpPlan, plan_http},
    package::{PackageConfig, emit_http},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn native_security_alternatives_credentials_and_declared_server_choices_are_explicit() {
    let witness: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut schemes = witness["securityCases"]["schemes"].clone();
    schemes["queryKey"] = json!({"type":"apiKey","in":"query","name":"api_key"});
    schemes["cookieKey"] = json!({"type":"apiKey","in":"cookie","name":"session"});
    let mut alternatives = witness["securityCases"]["security"].clone();
    alternatives
        .as_array_mut()
        .unwrap()
        .push(json!({"key":[],"queryKey":[],"cookieKey":[]}));
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Credential hooks and server choices","version":"1"},
            "servers":[{"url":"/{basePath}","name":"primary","variables":{"basePath":{"default":"v1","enum":["v1","v2"]}}},{"url":"../api","name":"relative"}],
            "components":{"securitySchemes":schemes},"paths":{
                "/secure":{"get":{"operationId":"secure","security":alternatives,"responses":{"204":{"description":"accepted"}}}},
                "/public":{"get":{"operationId":"publicCall","security":[],"responses":{"204":{"description":"accepted"}}}},
                "/implicit":{"get":{"operationId":"implicitCall","responses":{"204":{"description":"accepted"}}}}
            }
        }),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient, type ClientOptions} from './operations.js';
const options: ClientOptions={auth:{basic:{username:'Aladdin',password:'open sesame'},oauth:context=>({authorization:'DPoP caller-token'}),cookieKey:'cookie'}};
const client=createClient(options);
export async function calls(){await client.secure({}, {securityAlternative:0});await client.publicCall();}
// @ts-expect-error basic credentials are structured, not a bearer token
const wrongBasic: ClientOptions={auth:{basic:'secret'}};
// @ts-expect-error OAuth token type is caller-chosen; no implicit Bearer string
const wrongOAuth: ClientOptions={auth:{oauth:'secret'}};
// @ts-expect-error unknown source scheme names are rejected
const wrongName: ClientOptions={auth:{invented:'secret'}};
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError,operationMetadata} from './dist/operations.js';
const requests=[];
const server=createServer((request,response)=>{requests.push({url:request.url,headers:request.headers});response.writeHead(204);response.end();});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`;
let hooks=0;
const auth={token:'caller-token',key:'header-key',queryKey:'a&b=+',cookieKey:'a b+/=',basic:{username:'Aladdin',password:'open sesame'},
    oauth:context=>{
        hooks++;
        assert.equal(context.requirement.permissions.kind,'scopes');
        assert.equal(context.requirement.permissions.names[0].value,'read:items');
        assert.equal(context.requirement.credential.flows[0].token_url.value,'https://auth.example.test/token');
        assert.equal(context.requirement.credential.flows[0].scopes['read:items'].value,'Read items');
        assert.match(context.requirement.scheme.terminal.source.pointer,/securitySchemes\/oauth$/);
        assert.equal(context.signal.aborted,false);
        return {authorization:'DPoP caller-token'};
    },oidc:context=>{
        hooks++;
        assert.equal(context.requirement.credential.discovery_url.value,'https://auth.example.test/.well-known/openid-configuration');
        return {authorization:'Bearer caller-oidc'};
    }};
const client=createClient({auth,server:{index:0,documentURL:base+'/docs/openapi.json'}});
try {
    await client.secure();
    assert.equal(requests[0].headers.authorization,undefined,'first declared anonymous alternative sends no auth');
    await client.secure({}, {securityAlternative:1});
    assert.equal(requests[1].headers.authorization,'Bearer caller-token');assert.equal(requests[1].headers['x-key'],'header-key');
    const token=operationMetadata.secure.security.alternatives[1].requirements.find(r=>r.name==='token');
    assert.equal(token.permissions.kind,'roles');assert.equal(token.permissions.names[0].value,'reader');
    await client.secure({}, {securityAlternative:2});assert.equal(requests[2].headers.authorization,'DPoP caller-token');
    await client.secure({}, {securityAlternative:3});assert.equal(requests[3].headers.authorization,'Bearer caller-oidc');
    await client.secure({}, {securityAlternative:4});assert.equal(requests[4].headers.authorization,'Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==');
    await client.secure({}, {securityAlternative:5});
    assert.equal(requests[5].url,'/v1/secure?api_key=a%26b%3D%2B');assert.equal(requests[5].headers.cookie,'session=a%20b%2B%2F%3D');
    await client.publicCall();await client.implicitCall();assert.equal(hooks,2);
    assert.equal(requests[6].headers.authorization,undefined);assert.equal(requests[7].headers.authorization,undefined);
    await client.secure({}, {server:{index:0,variables:{basePath:'v2'},documentURL:base+'/docs/openapi.json'}});
    assert.equal(requests[8].url,'/v2/secure');
    await client.secure({}, {server:{name:'relative',documentURL:base+'/docs/openapi.json'}});
    assert.equal(requests[9].url,'/api/secure');
    const before=requests.length;
    for(const choice of [{index:7},{index:0,variables:{typo:'x'}},{index:0,variables:{basePath:'v3'}},{index:0,documentURL:'file:///spec.json'}]) {
        await assert.rejects(client.secure({}, {server:choice}),error=>isSdkError(error)&&error.kind==='request-representation');
    }
    await assert.rejects(client.secure({}, {securityAlternative:12}),error=>isSdkError(error)&&error.kind==='request-validation');
    const missing=createClient({serverURL:base});
    await assert.rejects(missing.secure({}, {securityAlternative:1}),error=>isSdkError(error)&&error.kind==='request-validation');
    const invalid=createClient({serverURL:base,auth:{oauth:()=>({authorization:'caller-token-without-type'})}});
    await assert.rejects(invalid.secure({}, {securityAlternative:2}),error=>isSdkError(error)&&error.kind==='request-validation');
    assert.equal(requests.length,before,'invalid policy never sends or retries a request');
    const mutable={username:'ü',password:'p',encoding:'utf-8'};
    const unicode=createClient({serverURL:base,auth:{basic:mutable}});mutable.password='changed';
    await unicode.secure({}, {securityAlternative:4});assert.equal(requests.at(-1).headers.authorization,'Basic w7w6cA==','constructor snapshots credentials');
} finally {await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn bodyless_default_and_range_statuses_preserve_native_discriminants() {
    let response = json!({"description":"actual status controls body presence","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"content":{"text/plain":{"schema":{"type":"string"}}}});
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Bodyless status discriminants","version":"1"},"paths":{
            "/default":{"get":{"operationId":"readDefault","responses":{"default":response.clone()}}},
            "/range":{"get":{"operationId":"readRange","responses":{"2XX":response}}}
        }}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient, type ReadDefaultSuccess, type ReadRangeSuccess} from './operations.js';
function narrow(response: ReadDefaultSuccess | ReadRangeSuccess) {
    const count: bigint = response.typedHeaders['X-Count'];
    if (response.status === 204) { const body: undefined = response.data; const media: null = response.contentType; }
    else if (response.status === 205) { const body: undefined = response.data; const media: null = response.contentType; }
    else { const body: string = response.data; const media: 'text/plain' = response.contentType; }
    if (response.status !== 204 && response.status !== 205) { const body: string = response.data; }
    // @ts-expect-error the body needs a status guard before use as text
    const unsafe: string = response.data;
    return count;
}
const status: ReadDefaultSuccess['status'] = 205;
// @ts-expect-error a default declaration does not put non-success statuses in the success union
const nonSuccess: ReadDefaultSuccess['status'] = 304;
export async function typedCalls() { const client = createClient(); narrow(await client.readDefault()); narrow(await client.readRange()); }
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
let status=201,missingHeader=false,calls=0;
const server=createServer((request,response)=>{
    calls++;
    response.writeHead(status,{'content-type':status===201?'text/plain':'not a media type',...(missingHeader?{}:{'X-Count':'9007199254740993'})});
    response.end(status===201?'typed body':undefined);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
    const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});
    for(const method of ['readDefault','readRange'])for(status of [201,204,205]) {
        const response=await client[method]();
        assert.equal(response.status,status);assert.equal(response.typedHeaders['X-Count'],9007199254740993n);
        assert.equal(response.data,status===201?'typed body':undefined);
        assert.equal(response.contentType,status===201?'text/plain':null);
    }
    missingHeader=true;status=205;
    for(const method of ['readDefault','readRange'])await assert.rejects(client[method](),error=>isSdkError(error)&&error.kind==='response-decoding');
    assert.equal(calls,8);
} finally { await new Promise(resolve=>server.close(resolve)); }
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn native_responses_dispatch_by_actual_status_media_and_typed_header_provenance() {
    let document = json!({"openapi":"3.2.0","info":{"title":"Response dispatch","version":"1"},"paths":{
        "/select":{"get":{"operationId":"selectResponse","parameters":[{"name":"mode","in":"query","schema":{"type":"string"}}],"responses":{
            "200":{"description":"exact","headers":{
                "X-Count":{"$ref":"#/components/headers/Count"},
                "X-Flags":{"schema":{"type":"array","items":{"type":"boolean"}}},
                "X-Meta":{"explode":true,"schema":{"type":"object","properties":{"zone":{"type":"string"},"count":{"type":"integer"}},"required":["zone","count"],"additionalProperties":false}},
                "X-Json":{"content":{"application/json":{"schema":{"type":"object","properties":{"name":{"type":"string"}}}}}},
                "Set-Cookie":{"schema":{"type":"string"}}
            },"links":{"next":{"operationId":"selectResponse","parameters":{"mode":"$response.body#/next","large":9007199254740993_u64},"requestBody":{"$ref":"literal-instance","schema":{"type":"integer"}}}},"content":{
                "application/json":{"schema":{"type":"object","required":["kind"],"properties":{"kind":{"type":"string"}},"additionalProperties":false}},
                "application/json;profile=a":{"schema":{"type":"object","required":["kind"],"properties":{"kind":{"const":"a"}},"additionalProperties":false}},
                "application/problem+json":{"schema":{"type":"object","properties":{"problem":{"type":"string"}},"required":["problem"]}},
                "text/plain":{"schema":{"type":"string"}},"application/*":{},"*/*":{}
            }},"2XX":{"description":"class bytes"},"default":{"description":"default bytes"}
        }}},
        "/precedence":{"get":{"operationId":"precedence","parameters":[{"name":"mode","in":"query","schema":{"type":"string"}}],"responses":{
            "200":{"description":"exact JSON","content":{"application/json":{"schema":{"type":"string"}}}},
            "2XX":{"description":"range text","content":{"text/plain":{"schema":{"type":"string"}}}},"default":{"description":"fallback"}
        }}},
        "/default":{"get":{"operationId":"defaultResponse","responses":{"default":{"description":"actual status decides success","content":{"text/plain":{"schema":{"type":"string"}}}}}}},
        "/head":{"head":{"operationId":"headMeta","responses":{"200":{"description":"headers only","headers":{"X-Count":{"$ref":"#/components/headers/Count"}},"content":{"application/json":{"schema":{"not":{}}}}}}}},
        "/empty":{"get":{"operationId":"empty","responses":{"204":{"description":"empty","content":{"application/json":{"schema":{"not":{}}}}}}}}
    },"components":{"headers":{"Count":{"required":true,"schema":{"type":"integer"}}}}});
    let plan = plan(document, HttpConfig::expanded());
    assert!(
        plan.protocol()
            .codec_roots()
            .iter()
            .all(|id| !id.pointer().contains("/links/")
                && !id.pointer().contains("/head/head/responses/200/content")
                && !id.pointer().contains("/empty/get/responses/204/content"))
    );
    let select = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "selectResponse")
        .unwrap();
    let count = select.protocol().responses()[0]
        .headers()
        .iter()
        .find(|header| header.name() == "X-Count")
        .unwrap();
    assert_eq!(
        count.source().terminal().source().pointer(),
        "/components/headers/Count"
    );
    native(
        &plan,
        r#"
import {createClient, type SelectResponseResponse200Headers, isSelectResponseApiError} from './operations.js';
const client=createClient();
export async function types() {
    const head=await client.headMeta();const absent: undefined=head.data;const count: bigint=head.typedHeaders['X-Count'];
    const response=await client.defaultResponse();if(response.status!==204&&response.status!==205){const text:string=response.data;}
    const headers:SelectResponseResponse200Headers={'X-Count':4n,'X-Flags':[true,false],'X-Meta':{zone:'eu',count:3n}};
    // @ts-expect-error source-required header is not optional
    const missing:SelectResponseResponse200Headers={};
    // @ts-expect-error header numeric conversion preserves bigint
    const wrong:SelectResponseResponse200Headers={'X-Count':4};
    return {absent,count,headers};
}
export function errorType(error:unknown){if(isSelectResponseApiError(error)){
    if(error.response.data!==undefined){
        // @ts-expect-error error byte snapshots have readonly indices
        error.response.data[0]=0;
    }
}}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError,isSelectResponseApiError} from './dist/operations.js';
let calls=0;
const server=createServer((request,response)=>{
    calls++;assert.equal(request.headers.cookie,undefined,'response Set-Cookie does not activate an ambient cookie jar');
    const url=new URL(request.url,'http://fixture');const mode=url.searchParams.get('mode')??'json';
    if(url.pathname==='/empty'){response.writeHead(204);response.end();return;}
    if(url.pathname==='/head'){response.writeHead(200,{'X-Count':'9007199254740993','content-type':'not a media type'});response.end();return;}
    if(url.pathname==='/default'){response.writeHead(201,{'content-type':'text/plain'});response.end('created');return;}
    if(url.pathname==='/precedence'){
        if(mode==='missing'){response.writeHead(200);response.end('"ok"');return;}
        response.writeHead(mode==='range'?201:200,{'content-type':mode==='invalid'?'application/json; charset=utf-8; CHARSET=latin1':mode==='charset'?'text/plain; charset=latin1':'text/plain'});response.end('text');return;
    }
    if(mode==='class'||mode==='error'){response.writeHead(mode==='class'?207:418);response.end(Buffer.from([0,255,42]));return;}
    const headers={'X-Count':'9007199254740993','X-Flags':'true,false','X-Meta':'count=3,zone=eu','X-Json':'{"name":"plain"}','Set-Cookie':'sid=fixture; HttpOnly'};
    if(mode==='missing-header')delete headers['X-Count'];
    if(mode==='bad-header')headers['X-Count']='3.5';
    if(mode==='cookies')headers['Set-Cookie']=['a=1','b=2'];
    let media='Application/JSON; charset=UTF-8', body='{"kind":"base"}';
    if(mode==='profile'){media='application/json;profile=a; charset="UTF-8"';body='{"kind":"a"}';}
    if(mode==='bad-profile'){media='application/json;profile=a';body='{"kind":"wrong"}';}
    if(mode==='case-profile'){media='application/json;profile=A';body='{"kind":"base"}';}
    if(mode==='problem'){media='application/problem+json';body='{"problem":"explicit"}';}
    if(mode==='text'){media='text/plain; charset=utf-8';body='snow 雪';}
    if(mode==='bytes'){media='application/pdf';body=Buffer.from([0,255,128,13,10]);}
    if(mode==='any'){media='image/png';body=Buffer.from([0,255,128,13,10]);}
    response.writeHead(200,{...headers,'content-type':media});response.end(body);
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});
try {
    const json=await client.selectResponse();assert.equal(json.data.kind,'base');assert.equal(json.contentType,'application/json');
    assert.equal(json.typedHeaders['X-Count'],9007199254740993n);assert.deepEqual(json.typedHeaders['X-Flags'],[true,false]);
    assert.equal(json.typedHeaders['X-Meta'].count,3n);assert.equal(json.typedHeaders['X-Meta'].zone,'eu');assert.equal(json.typedHeaders['X-Json'].name,'plain');
    assert.equal(json.links.next.target.value.value,'selectResponse');assert.equal(json.links.next.request_body.value.$ref,'literal-instance');
    assert.equal(json.links.next.parameters.large.value.toString(),'9007199254740993');assert.equal(calls,1,'links do not invoke operations');
    const profile=await client.selectResponse({mode:'profile'});assert.equal(profile.contentType,'application/json;profile=a');assert.equal(profile.data.kind,'a');
    assert.equal((await client.selectResponse({mode:'case-profile'})).contentType,'application/json','non-charset media parameter values are case sensitive');
    assert.equal((await client.selectResponse({mode:'problem'})).data.problem,'explicit');assert.equal((await client.selectResponse({mode:'text'})).data,'snow 雪');
    for(const mode of ['bytes','any'])assert.deepEqual([...(await client.selectResponse({mode})).data],[0,255,128,13,10]);
    const range=await client.selectResponse({mode:'class'});assert.equal(range.status,207);assert.deepEqual([...range.data],[0,255,42]);
    await assert.rejects(client.selectResponse({mode:'error'}),error=>{
        assert.ok(isSelectResponseApiError(error));assert.equal(error.response.status,418);assert.deepEqual([...error.response.data],[0,255,42]);
        error.response.data[0]=7;assert.equal(error.response.data[0],0,'error byte mutation cannot change its validated snapshot');return true;
    });
    for(const mode of ['missing-header','bad-header','bad-profile','cookies'])await assert.rejects(client.selectResponse({mode}),error=>isSdkError(error)&&error.kind==='response-decoding');
    for(const mode of ['exact','missing','invalid','charset'])await assert.rejects(client.precedence({mode}),error=>isSdkError(error)&&error.kind==='unexpected-response');
    assert.equal((await client.precedence({mode:'range'})).data,'text');
    const fallback=await client.defaultResponse();assert.equal(fallback.status,201);assert.equal(fallback.data,'created');
    const head=await client.headMeta();assert.equal(head.data,undefined);assert.equal(head.typedHeaders['X-Count'],9007199254740993n);
    assert.equal((await client.empty()).data,undefined);
    let cancelled=false,pulled=0;
    const body=new ReadableStream({pull(){pulled++;},cancel(){cancelled=true;}});
    const fake=new Response(body,{status:200});Object.defineProperty(fake,'status',{value:204});
    const before=pulled;const empty=createClient({serverURL:'https://example.test',fetch:async()=>fake});
    assert.equal((await empty.empty()).data,undefined);assert.equal(cancelled,true);assert.ok(pulled<=before+1,'204 bytes are never decoded');
} finally {await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn forms_and_named_multipart_send_literal_values_framing_headers_and_bounded_binary_parts() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut document = fixture["multipart"].clone();
    document["paths"]["/forms"] = fixture["form"]["paths"]["/forms"].clone();
    document["paths"]["/forms"]["post"]["operationId"] = json!("submitForm");
    document["paths"]["/forms"]["post"]["requestBody"]["required"] = json!(true);
    let media = document["paths"]["/upload"]["post"]["requestBody"]["content"]
        .as_object_mut()
        .unwrap()
        .remove("multipart/form-data")
        .unwrap();
    document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data;boundary=boundary-example"] =
        media;
    let media = &mut document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data;boundary=boundary-example"];
    media["schema"]["minProperties"] = json!(2);
    media["schema"]["maxProperties"] = json!(4);
    media["schema"]["properties"]["files"] =
        json!({"type":"array","minItems":1,"maxItems":2,"items":{}});
    media["encoding"]["files"] = json!({"contentType":"application/octet-stream"});
    let plan = plan(document, HttpConfig::expanded());
    assert!(plan.protocol().codec_roots().iter().all(|root| {
        !root
            .pointer()
            .ends_with("/multipart~1form-data;boundary=boundary-example/schema")
            && !root.pointer().ends_with("/properties/file")
            && !root.pointer().ends_with("/properties/files/items")
    }));
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function values(){
    await client.submitForm({body:{id:'snow 雪',address:{city:'a+b c'},tags:['a +','b'],codes:[1n,9007199254740993n]}});
    await client.upload({body:{file:{data:new Uint8Array([0,255]),headers:{'X-Part-Id':'part-1'},contentType:'image/png'},metadata:{title:'native'},files:[new Uint8Array([0])]}});
    // @ts-expect-error binary data never enters the JSON/string model
    await client.upload({body:{file:{data:'file-path',headers:{'X-Part-Id':'part-1'},contentType:'image/png'},metadata:{}}});
    // @ts-expect-error the required part header is present in the native input type
    await client.upload({body:{file:{data:new Uint8Array(),contentType:'image/png'},metadata:{}}});
    // @ts-expect-error selecting a part media type is required for multiple choices
    await client.upload({body:{file:{data:new Uint8Array(),headers:{'X-Part-Id':'part-1'}},metadata:{}}});
    // @ts-expect-error aggregate required field metadata cannot be omitted
    await client.upload({body:{file:{data:new Uint8Array(),headers:{'X-Part-Id':'part-1'},contentType:'image/png'}}});
}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const requests=[];
const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{requests.push({url:request.url,headers:request.headers,body:Buffer.concat(chunks)});response.writeHead(request.url==='/forms'?200:204);response.end();});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`;
const client=createClient({serverURL});
const binary=new Uint8Array([0,255,128,13,10,65]);
const body={file:{data:binary,headers:{'X-Part-Id':'part-1'},contentType:'image/png',filename:'data.bin'},metadata:{title:'snow 雪'},labels:['one','two'],files:[new Uint8Array([0,1]),new Uint8Array([254,255])]};
try {
    await client.submitForm({body:{id:'snow 雪',address:{city:'a+b c'},tags:['a +','b'],codes:[1n,9007199254740993n]}});
    assert.equal(requests[0].headers['content-type'],'application/x-www-form-urlencoded');
    assert.equal(requests[0].body.toString(),'address=%7B%22city%22%3A%22a%2Bb+c%22%7D&codes=1,9007199254740993&id=snow+%E9%9B%AA&tags=a+%2B&tags=b');
    await client.upload({body});
    const upload=requests[1];assert.equal(upload.headers['content-type'],'multipart/form-data;boundary=boundary-example');
    const form=await new Response(upload.body,{headers:{'content-type':upload.headers['content-type']}}).formData();
    assert.deepEqual([...new Uint8Array(await form.get('file').arrayBuffer())],[0,255,128,13,10,65]);
    assert.equal(form.get('file').name,'data.bin');assert.equal(form.get('file').type,'image/png');
    assert.deepEqual(form.getAll('labels'),['one','two']);assert.equal(form.get('metadata'),'{"title":"snow 雪"}');
    assert.deepEqual(await Promise.all(form.getAll('files').map(async file=>[...new Uint8Array(await file.arrayBuffer())])),[[0,1],[254,255]]);
    const expectedPrefix=Buffer.from('--boundary-example\r\nX-Part-Id: part-1\r\nContent-Disposition: form-data; name="file"; filename="data.bin"\r\nContent-Type: image/png\r\n\r\n');
    assert.deepEqual(upload.body.subarray(0,expectedPrefix.length),expectedPrefix);
    assert.deepEqual(upload.body.subarray(expectedPrefix.length,expectedPrefix.length+binary.length),Buffer.from(binary));
    assert.ok(upload.body.toString().endsWith('\r\n--boundary-example--\r\n'));
    const before=requests.length;
    for(const bad of [{metadata:{}},{...body,extra:'forbidden'},{...body,labels:[]},{...body,labels:['1','2','3','4']},{...body,files:[binary,binary,binary]},{...body,file:{...body.file,headers:{}}},{...body,file:{...body.file,headers:{'X-Part-Id':'x\r\nInjected: yes'}}},{...body,file:{...body.file,contentType:'text/plain'}}]){
        await assert.rejects(client.upload({body:bad}),error=>isSdkError(error)&&['request-validation','request-representation'].includes(error.kind));
    }
    await assert.rejects(createClient({serverURL,maxPartBytes:1}).upload({body}),error=>isSdkError(error)&&error.kind==='resource-limit');
    await assert.rejects(createClient({serverURL,maxRequestBytes:16}).upload({body}),error=>isSdkError(error)&&error.kind==='resource-limit');
    await assert.rejects(client.upload({body:{...body,file:{...body.file,data:new TextEncoder().encode('boundary-example')}}}),error=>isSdkError(error)&&error.kind==='request-representation');
    assert.equal(requests.length,before,'invalid multipart values never reach the transport');
} finally {await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn positional_multipart_and_form_responses_decode_independent_literal_bytes_and_cardinalities() {
    let multipart = json!({"schema":{"type":"object","required":["file","meta","quota"],"minProperties":3,"maxProperties":4,"properties":{
        "file":{},"meta":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"],"additionalProperties":false},
        "tags":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string"}}
    },"additionalProperties":{"type":"integer"}},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer","minimum":0,"maximum":99}}}}}});
    let form = json!({"schema":{"type":"object","required":["id"],"properties":{"id":{"type":"string"},"map":{"type":"object","additionalProperties":{"type":"integer"}},"tags":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string"}}},"additionalProperties":false}});
    let positional = json!({"schema":{"type":"array","minItems":1,"maxItems":3,"prefixItems":[{"type":"string"}],"items":{"type":"object","required":["n"],"properties":{"n":{"type":"integer"}},"additionalProperties":false}},"prefixEncoding":[{"contentType":"text/plain"}],"itemEncoding":{"contentType":"application/json","headers":{"X-Item":{"required":true,"schema":{"type":"boolean"}}}}});
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Literal response MIME witnesses","version":"1"},"paths":{
            "/download":{"get":{"operationId":"download","parameters":[{"name":"mode","in":"query","schema":{"type":"string"}}],"responses":{"200":{"description":"named parts","content":{"multipart/form-data;boundary=native-boundary":multipart}}}}},
            "/form":{"get":{"operationId":"readForm","parameters":[{"name":"mode","in":"query","schema":{"type":"string"}}],"responses":{"200":{"description":"form values","content":{"application/x-www-form-urlencoded":form}}}}},
            "/extras":{"post":{"operationId":"sendExtras","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","required":["id","quota"],"minProperties":2,"maxProperties":3,"properties":{"id":{"type":"string"}},"additionalProperties":{"type":"integer"}}}}},"responses":{"204":{"description":"accepted"}}}},
            "/ordered":{"post":{"operationId":"ordered","requestBody":{"required":true,"content":{"multipart/mixed;boundary=ordered":positional.clone()}},"responses":{"200":{"description":"positional parts","content":{"multipart/mixed;boundary=ordered":positional}}}}}
        }}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function nativeValues(){
    await client.sendExtras({body:{id:'native',additionalFields:{quota:3n,balance:4n}}});
    const parts=await client.ordered({body:['intro',{data:{n:9007199254740993n},headers:{'X-Item':true}}]});
    const first:string=parts.data[0];
    const named=await client.download();const bytes:Uint8Array=named.data.file.data;const quota:bigint=named.data.additionalFields.quota;
    // @ts-expect-error positional prefix is text, not an object part
    await client.ordered({body:[{n:1n}]});
    // @ts-expect-error per-item headers are required by itemEncoding
    await client.ordered({body:['intro',{data:{n:1n}}]});
    // @ts-expect-error required undeclared-property names remain required in the extras bag
    await client.sendExtras({body:{id:'native',additionalFields:{balance:1n}}});
    return {first,bytes,quota};
}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const prefix='--native-boundary\r\nContent-Disposition: form-data; name="file"; filename="file.bin"\r\nContent-Type: application/octet-stream\r\nX-Part: 7\r\n\r\n';
const suffix='\r\n--native-boundary\r\nContent-Disposition: form-data; name="meta"\r\nContent-Type: application/json\r\n\r\n{"n":9007199254740993}\r\n--native-boundary\r\nContent-Disposition: form-data; name="quota"\r\nContent-Type: text/plain\r\n\r\n5\r\n--native-boundary--\r\n';
const multipart=Buffer.concat([Buffer.from(prefix),Buffer.from([0,255,128,10]),Buffer.from(suffix)]);
const sent='--ordered\r\nContent-Type: text/plain\r\n\r\nintro\r\n--ordered\r\nX-Item: true\r\nContent-Type: application/json\r\n\r\n{"n":9007199254740993}\r\n--ordered--\r\n';
const received='ignored preamble\r\n--ordered\r\n\r\nresponse\r\n--ordered\r\nX-Item: false\r\nContent-Type: application/json\r\n\r\n{"n":9007199254740995}\r\n--ordered--\r\nignored epilogue';
const requests=[];
const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{
    const url=new URL(request.url,'http://fixture');const mode=url.searchParams.get('mode');const body=Buffer.concat(chunks);requests.push({url:request.url,body});
    if(url.pathname==='/extras'){response.writeHead(204);response.end();return;}
    if(url.pathname==='/ordered'){response.writeHead(200,{'content-type':'multipart/mixed;boundary=ordered'});response.end(received);return;}
    if(url.pathname==='/form'){response.writeHead(200,{'content-type':'application/x-www-form-urlencoded'});response.end(mode==='duplicate'?'id=a&id=b':mode==='bad-item'?'id=a&map=%7B%22n%22%3A%22bad%22%7D':'id=hello+%E9%9B%AA&map=%7B%22n%22%3A9007199254740993%7D&tags=a&tags=b');return;}
    let bytes=multipart;
    if(mode==='missing-header')bytes=Buffer.concat([Buffer.from(prefix.replace('X-Part: 7\r\n','')),Buffer.from([0,255,128,10]),Buffer.from(suffix)]);
    if(mode==='missing-field')bytes=Buffer.from('--native-boundary\r\nContent-Disposition: form-data; name="quota"\r\nContent-Type: text/plain\r\n\r\n5\r\n--native-boundary--\r\n');
    if(mode==='truncated')bytes=multipart.subarray(0,multipart.length-15);
    response.writeHead(200,{'content-type':'multipart/form-data; boundary=native-boundary'});response.end(bytes);
});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
try {
    const named=await client.download();assert.deepEqual([...named.data.file.data],[0,255,128,10]);assert.equal(named.data.file.filename,'file.bin');assert.equal(named.data.file.headers['X-Part'],7);
    assert.equal(named.data.meta.n,9007199254740993n);assert.equal(named.data.additionalFields.quota,5n);
    const form=await client.readForm();assert.equal(form.data.id,'hello 雪');assert.equal(form.data.map.n,9007199254740993n);assert.deepEqual(form.data.tags,['a','b']);
    await client.sendExtras({body:{id:'native',additionalFields:{quota:3n,balance:4n}}});assert.equal(requests.at(-1).body.toString(),'balance=4&id=native&quota=3');
    const ordered=await client.ordered({body:['intro',{data:{n:9007199254740993n},headers:{'X-Item':true}}]});assert.equal(requests.at(-1).body.toString(),sent);
    assert.equal(ordered.data[0],'response');assert.equal(ordered.data[1].data.n,9007199254740995n);assert.equal(ordered.data[1].headers['X-Item'],false);
    for(const mode of ['missing-header','missing-field','truncated'])await assert.rejects(client.download({mode}),error=>isSdkError(error)&&error.kind==='response-decoding');
    for(const mode of ['duplicate','bad-item'])await assert.rejects(client.readForm({mode}),error=>isSdkError(error)&&error.kind==='response-decoding');
    await assert.rejects(createClient({serverURL,maxPartBytes:3}).download(),error=>isSdkError(error)&&error.kind==='resource-limit');
    const before=requests.length;
    await assert.rejects(client.ordered({body:[]}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.ordered({body:['intro',{}, {}, {}]}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.ordered({body:['intro',{data:{n:'bad'},headers:{'X-Item':true}}]}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.sendExtras({body:{id:'a',additionalFields:{quota:1n,id:2n}}}),error=>isSdkError(error)&&error.kind==='request-validation');
    assert.equal(requests.length,before);
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn native_sse_and_json_lines_preserve_envelopes_exact_items_backpressure_and_cleanup() {
    let event = json!({"type":"object","required":["data"],"properties":{"data":{"type":"string","contentMediaType":"application/json","contentSchema":{"type":"object","required":["notAutomaticallyValidated"]}},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false});
    let item = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Native sequential framing","version":"1"},"paths":{
            "/events":{"get":{"operationId":"events","responses":{"200":{"description":"events","content":{"text/event-stream":{"itemSchema":event.clone()}}}}}},
            "/lines":{"get":{"operationId":"lines","responses":{"200":{"description":"lines","content":{"application/x-ndjson":{"itemSchema":item.clone()}}}}}},
            "/send-events":{"post":{"operationId":"sendEvents","requestBody":{"required":true,"content":{"text/event-stream":{"itemSchema":event}}},"responses":{"204":{"description":"accepted"}}}},
            "/send-lines":{"post":{"operationId":"sendLines","requestBody":{"required":true,"content":{"application/jsonl":{"itemSchema":item}}},"responses":{"204":{"description":"accepted"}}}}
        }}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function events(){
    const response=await client.events();
    for await(const event of response.data){const text:string=event.data;const retry:bigint|undefined=event.retry;
        // @ts-expect-error data is the SSE string, not invented nested JSON
        const nested=event.data.value;
    }
    await client.sendEvents({body:[{data:'alpha\nbeta',event:'update',retry:5n,id:'one'}]});
    await client.sendLines({body:(async function*(){yield {value:9007199254740993n};})()});
    // @ts-expect-error itemSchema integer uses bigint, independently of framing
    await client.sendLines({body:[{value:'bad'}]});
    // @ts-expect-error event data is a string, including JSON-looking data
    await client.sendEvents({body:[{data:{value:1}}]});
}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const encoder=new TextEncoder();
const eventBytes=encoder.encode('\uFEFF: keepalive\r\nevent: update\rid: first\ndata: snow 雪\r\ndata: second\rretry: 0005\r\nunknown: ignored\r\n\r\ndata: {"x":1}\n\ndata: [DONE]\n\nid: bad\0value\nretry: 5x\ndata:\n\ndata: incomplete');
const lineBytes=encoder.encode('{"value":9007199254740993}\r\n{"value":2}\n{"value":3}');
const requests=[];
const server=createServer((request,response)=>{
    if(request.url.startsWith('/send-')){const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{requests.push({url:request.url,body:Buffer.concat(chunks).toString(),media:request.headers['content-type']});response.writeHead(204);response.end();});return;}
    response.writeHead(200,{'content-type':request.url==='/events'?'text/event-stream':'application/x-ndjson'});
    const bytes=request.url==='/events'?eventBytes:lineBytes;
    for(let offset=0;offset<bytes.length;offset+=3)response.write(bytes.subarray(offset,offset+3));response.end();
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
const collect=async iterable=>{const values=[];for await(const value of iterable)values.push(value);return values;};
const expectKind=kind=>error=>isSdkError(error)&&error.kind===kind;
try {
    const events=await collect((await client.events()).data);
    assert.equal(events.length,4,'SSE EOF does not dispatch an unfinished event');
    assert.equal(events[0].data,'snow 雪\nsecond');assert.equal(events[0].event,'update');assert.equal(events[0].id,'first');assert.equal(events[0].retry,5n);
    assert.equal(events[1].data,'{"x":1}');assert.equal(events[2].data,'[DONE]');assert.equal(events[3].data,'');assert.equal(events[3].retry,undefined);
    assert.deepEqual((await collect((await client.lines()).data)).map(value=>value.value),[9007199254740993n,2n,3n]);
    await client.sendEvents({body:[{data:'alpha\nbeta',event:'update',retry:5n,id:'one'}]});
    assert.equal(requests[0].media,'text/event-stream');assert.equal(requests[0].body,'event: update\nid: one\nretry: 5\ndata: alpha\ndata: beta\n\n');
    await client.sendLines({body:(async function*(){yield {value:9007199254740993n};yield {value:2n};})()});
    assert.equal(requests[1].media,'application/jsonl');assert.equal(requests[1].body,'{"value":9007199254740993}\n{"value":2}\n');

    // A one-byte transport split fixes every UTF-8 and CR/LF boundary independently
    // of network coalescing; it still goes through the public generated operation.
    const split=new ReadableStream({start(controller){for(const byte of eventBytes)controller.enqueue(new Uint8Array([byte]));controller.close();}});
    const splitClient=createClient({serverURL,fetch:async()=>new Response(split,{headers:{'content-type':'text/event-stream'}})});
    assert.equal((await collect((await splitClient.events()).data))[0].data,'snow 雪\nsecond');assert.equal(split.locked,false);

    const observedSignal=()=>{
        const controller=new AbortController();let listeners=0;
        const add=controller.signal.addEventListener.bind(controller.signal),remove=controller.signal.removeEventListener.bind(controller.signal);
        controller.signal.addEventListener=(kind,...args)=>{if(kind==='abort')listeners++;return add(kind,...args);};
        controller.signal.removeEventListener=(kind,...args)=>{if(kind==='abort')listeners--;return remove(kind,...args);};
        return {controller,listeners:()=>listeners};
    };
    let pulls=0,cancels=0;
    const body=new ReadableStream({pull(controller){pulls++;controller.enqueue(encoder.encode('data: ordinary\n\n'));},cancel(){cancels++;}});
    const watched=observedSignal();
    const lazy=createClient({serverURL,fetch:async()=>new Response(body,{headers:{'content-type':'text/event-stream'}})});
    const response=await lazy.events({}, {signal:watched.controller.signal});
    assert.ok(pulls<=1,'iterator does not eagerly drain the transport');assert.equal(watched.listeners(),1);
    for await(const event of response.data){assert.equal(event.data,'ordinary');break;}
    assert.equal(cancels,1);assert.equal(body.locked,false);assert.equal(watched.listeners(),0);assert.ok(pulls<=2,'one-item demand has bounded read-ahead');

    for(const begin of [false,true]){
        const watched=observedSignal();let cancelled=false;
        const pending=new ReadableStream({pull(){return new Promise(()=>{});},cancel(){cancelled=true;}});
        const waiting=createClient({serverURL,fetch:async()=>new Response(pending,{headers:{'content-type':'text/event-stream'}})});
        const iterator=(await waiting.events({}, {signal:watched.controller.signal})).data[Symbol.asyncIterator]();
        if(begin){const next=iterator.next();watched.controller.abort();await assert.rejects(next,expectKind('cancelled'));}
        else {await iterator.return();}
        assert.equal(cancelled,true);assert.equal(pending.locked,false);assert.equal(watched.listeners(),0);
    }
    let stopped=false;
    const idleBody=new ReadableStream({cancel(){stopped=true;}}),idleAbort=new AbortController();
    const idle=await createClient({serverURL,fetch:async()=>new Response(idleBody,{headers:{'content-type':'text/event-stream'}})}).events({}, {signal:idleAbort.signal});
    idleAbort.abort();assert.equal(stopped,true);assert.equal(idleBody.locked,false);await assert.rejects(idle.data[Symbol.asyncIterator]().next(),expectKind('cancelled'));

    for(const [options,payload,media,operation,kind] of [
        [{maxStreamItemBytes:8},'data: too long\n\n','text/event-stream','events','resource-limit'],
        [{maxStreamBufferBytes:1},'data: x\n\n','text/event-stream','events','resource-limit'],
        [{maxStreamItems:1},'data: one\n\ndata: two\n\n','text/event-stream','events','resource-limit'],
        [{maxResponseBytes:8},'data: one\n\n','text/event-stream','events','resource-limit'],
        [{},'{"value":"wrong"}\n','application/x-ndjson','lines','response-decoding'],
        [{},'{"value":1}\n\n','application/x-ndjson','lines','response-decoding'],
        [{},'[DONE]\n','application/x-ndjson','lines','response-decoding']
    ]){
        let cancelled=false;const watched=observedSignal();
        const stream=new ReadableStream({start(controller){controller.enqueue(encoder.encode(payload));},cancel(){cancelled=true;}});
        const response=await createClient({serverURL,...options,fetch:async()=>new Response(stream,{headers:{'content-type':media}})})[operation]({}, {signal:watched.controller.signal});
        await assert.rejects(collect(response.data),expectKind(kind));assert.equal(cancelled,true);assert.equal(stream.locked,false);assert.equal(watched.listeners(),0);
    }
    let returned=0;
    const invalidItems={ [Symbol.asyncIterator](){return this;},async next(){return {value:{value:'wrong'},done:false};},async return(){returned++;return {done:true};}};
    await assert.rejects(client.sendLines({body:invalidItems}),expectKind('request-validation'));assert.equal(returned,1);assert.equal(requests.length,2);

    let resolve,lateCancelled=false;const abort=new AbortController();
    const waiting=createClient({serverURL,fetch:()=>new Promise(r=>{resolve=r;})}).events({}, {signal:abort.signal});
    await new Promise(r=>setTimeout(r,0));abort.abort();await assert.rejects(waiting,expectKind('cancelled'));
    const late=new ReadableStream({cancel(){lateCancelled=true;}});resolve(new Response(late,{headers:{'content-type':'text/event-stream'}}));
    await new Promise(r=>setTimeout(r,0));assert.equal(lateCancelled,true,'late headers from an uncooperative transport do not leak a body');
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn request_media_choices_preserve_native_values_and_cannot_bypass_specific_codecs() {
    let plan = plan(
        json!({"openapi":"3.1.2","info":{"title":"Explicit media choices","version":"1"},"paths":{
            "/media":{"post":{"operationId":"sendMedia","requestBody":{"required":true,"content":{
                "application/json":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false}},
                "application/problem+json":{"schema":{"type":"string"}},"text/plain":{"schema":{"type":"integer"}},"application/*":{},"*/*":{}
            }},"responses":{"200":{"description":"same concrete media","content":{"application/json":{"schema":{"type":"object","properties":{"name":{"type":"string"}}}},"application/problem+json":{"schema":{"type":"string"}},"text/plain":{"schema":{"type":"integer"}},"application/*":{},"*/*":{}}}}}}
        }}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function nativeMedia(){
    await client.sendMedia({body:{contentType:'application/json',data:{name:'native'}}});
    await client.sendMedia({body:{contentType:'text/plain',data:9007199254740993n}});
    const response=await client.sendMedia({body:{mediaType:'application/*',contentType:'application/pdf',data:new Uint8Array([0,255])}});
    if(response.mediaType==='application/*'){const bytes:Uint8Array=response.data;return bytes;}
    // @ts-expect-error a wildcard request needs an explicit source-range discriminator
    await client.sendMedia({body:{contentType:'application/json',data:new Uint8Array([0])}});
    // @ts-expect-error text uses the source integer model, not a string or number
    await client.sendMedia({body:{contentType:'text/plain',data:'3'}});
}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const requests=[];
const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{const body=Buffer.concat(chunks);requests.push({media:request.headers['content-type'],body});response.writeHead(200,{'content-type':request.headers['content-type']});response.end(body);});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
try {
    assert.equal((await client.sendMedia({body:{contentType:'application/json',data:{name:'native'}}})).data.name,'native');
    assert.equal(requests[0].body.toString(),'{"name":"native"}');
    assert.equal((await client.sendMedia({body:{contentType:'application/problem+json',data:'problem'}})).data,'problem');
    assert.equal((await client.sendMedia({body:{contentType:'text/plain',data:9007199254740993n}})).data,9007199254740993n);assert.equal(requests[2].body.toString(),'9007199254740993');
    for(const [mediaType,contentType] of [['application/*','application/pdf'],['*/*','image/png']]){
        const response=await client.sendMedia({body:{mediaType,contentType,data:new Uint8Array([0,255,128])}});
        assert.deepEqual([...response.data],[0,255,128]);assert.equal(response.contentType,contentType);assert.equal(response.mediaType,mediaType);
    }
    const before=requests.length;
    for(const body of [{mediaType:'application/*',contentType:'application/json',data:new Uint8Array([0])},{contentType:'application/json',data:{}},{mediaType:'*/*',contentType:'*/*',data:new Uint8Array()},{contentType:'text/plain;charset=latin1',data:3n},{contentType:'application/json;charset=x;charset=y',data:{name:'native'}}]){
        await assert.rejects(client.sendMedia({body}),error=>isSdkError(error)&&['request-validation','request-representation'].includes(error.kind));
    }
    await assert.rejects(createClient({serverURL,maxRequestBytes:2}).sendMedia({body:{mediaType:'application/*',contentType:'application/pdf',data:new Uint8Array([0,255,128])}}),error=>isSdkError(error)&&error.kind==='resource-limit');
    assert.equal(requests.length,before);
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn indexed_standard_methods_and_unnamed_operations_keep_exact_native_transport_tokens() {
    let mut document =
        json!({"openapi":"3.2.0","info":{"title":"Method inventory","version":"1"},"paths":{}});
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
    ] {
        document["paths"][format!("/methods/{method}")] =
            json!({method:{"responses":{"200":{"description":"recorded"}}}});
    }
    let plan = plan(document, HttpConfig::expanded());
    assert_eq!(
        plan.operations().len(),
        9,
        "all indexed standard methods, including QUERY"
    );
    let operations=plan.operations().iter().map(|operation|json!({"name":operation.function_name,"method":operation.method,"path":operation.path})).collect::<Vec<_>>();
    assert!(
        plan.operations()
            .iter()
            .all(|operation| operation.protocol().operation_id().is_none())
    );
    native(&plan,"import {createClient} from './operations.js';\nexport const client=createClient();\n",&r#"
import assert from 'node:assert/strict';
import {createServer,request as send} from 'node:http';
import * as operations from './dist/operations.js';
const methods=__METHODS__,requests=[];
const server=createServer((request,response)=>{requests.push({method:request.method,url:request.url});response.writeHead(200);response.end(request.method==='HEAD'?undefined:Buffer.from([0,255]));});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`;
const native=operations.createClient({serverURL});
// Fetch forbids TRACE. The explicit Fetch-shaped transport keeps the actual
// source token and is independently witnessed against a real HTTP server.
const traceFetch=(url,init)=>new Promise((resolve,reject)=>{const request=send(url,{method:init.method,headers:Object.fromEntries(new Headers(init.headers))},response=>{const bytes=[];response.on('data',chunk=>bytes.push(chunk));response.on('end',()=>resolve(new Response(Buffer.concat(bytes),{status:response.statusCode,headers:response.headers})));});request.on('error',reject);request.end();});
try{
    for(const operation of methods){
        assert.equal(typeof native[operation.name],'function');
        const result=await operations[operation.name]({serverURL,...(operation.method==='TRACE'?{fetch:traceFetch}:{})});
        assert.deepEqual(requests.at(-1),{method:operation.method,url:operation.path});
        if(operation.method==='HEAD')assert.equal(result.data,undefined);else assert.deepEqual([...result.data],[0,255]);
    }
    const trace=methods.find(operation=>operation.method==='TRACE'),before=requests.length;
    await assert.rejects(native[trace.name](),error=>operations.isSdkError(error)&&error.kind==='request-representation');
    assert.equal(requests.length,before,'native Fetch refusal occurs before transport');
}finally{await new Promise(resolve=>server.close(resolve));}
"#.replace("__METHODS__",&serde_json::to_string(&operations).unwrap()));
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn legacy_binary_markers_are_explicit_and_never_replace_json_codec_inputs() {
    let mut document = json!({"openapi":"3.1.2","info":{"title":"Explicit binary compatibility","version":"1"},"paths":{
        "/bytes":{"post":{"operationId":"bytes","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"raw","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}
    }});
    let source = contract(document.clone());
    let selected = source
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let errors = plan_http(source, &selected, HttpConfig::expanded()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-binary-legacy-marker"
                && error.source.pointer().ends_with("/schema/format")
                && !error.at.is_empty())
    );
    let config = HttpConfig {
        legacy_binary_string: true,
        ..HttpConfig::expanded()
    };
    let legacy = plan(document.clone(), config);
    assert!(legacy.protocol().codec_roots().is_empty());
    assert!(
        legacy
            .protocol()
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "http-compatibility-profile")
    );
    native(
        &legacy,
        r#"
import {createClient} from './operations.js';
export async function binary(){const client=createClient();const response=await client.bytes({body:new Uint8Array([0,255])});const bytes:Uint8Array=response.data;
// @ts-expect-error compatibility is a byte context, not a string model
await client.bytes({body:'file-path'});return bytes;}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const server=createServer((request,response)=>{const bytes=[];request.on('data',chunk=>bytes.push(chunk));request.on('end',()=>{const body=Buffer.concat(bytes);assert.deepEqual([...body],[0,255,128,10]);response.writeHead(200,{'content-type':'application/octet-stream'});response.end(Buffer.from([0,254,129,13]));});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const serverURL=`http://127.0.0.1:${server.address().port}`;let invoked=0;
    const bytes=new Uint8Array([0,255,128,10]);Object.defineProperty(bytes,'byteLength',{value:0});bytes.slice=()=>{invoked++;return new Uint8Array([9]);};
    await assert.rejects(createClient({serverURL,maxRequestBytes:1}).bytes({body:bytes}),error=>isSdkError(error)&&error.kind==='resource-limit');
    const response=await createClient({serverURL}).bytes({body:bytes});assert.deepEqual([...response.data],[0,254,129,13]);assert.equal(invoked,0,'byte snapshots use intrinsic length and copy operations');
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
    document["openapi"] = json!("3.0.4");
    let oas30 = plan(document, HttpConfig::expanded());
    assert!(oas30.protocol().codec_roots().is_empty());
    assert!(
        !oas30
            .protocol()
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "http-compatibility-profile")
    );
}

#[test]
#[ignore = "requires an isolated Chromium executable; set SUSPECT_CHROMIUM if not installed at the macOS default"]
fn browser_fetch_executes_native_bytes_multipart_events_credentials_and_cancellation() {
    let whole_query = |media: &str, schema: Value| json!([{"name":"queryValue","in":"querystring","required":true,"content":{media:{"schema":schema}}}]);
    let styled = json!({"schema":{"type":"object","required":["values","map"],"additionalProperties":false,"properties":{"values":{"type":"array","items":{"type":"string"}},"map":{"type":"object","required":["code"],"properties":{"code":{"type":"integer"}},"additionalProperties":false}}},"encoding":{"values":{"style":"form","explode":true},"map":{"style":"form","explode":true}}});
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Browser native protocol","version":"1"},"servers":[{"url":"/api"}],
            "components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"},"session":{"type":"apiKey","in":"cookie","name":"session"}}},
            "paths":{
                "/number":{"get":{"operationId":"readNumber","security":[{"token":[]}],"responses":{"200":{"description":"exact","headers":{"X-Count":{"required":true,"schema":{"type":"integer","minimum":0,"maximum":99}}},"content":{"application/json":{"schema":{"type":"object","required":["amount"],"properties":{"amount":{"type":"number"}},"additionalProperties":false}}}}}}},
                "/upload":{"post":{"operationId":"upload","requestBody":{"required":true,"content":{"application/octet-stream":{}}},"responses":{"200":{"description":"bytes","content":{"application/octet-stream":{}}}}}},
                "/parts":{"post":{"operationId":"parts","requestBody":{"required":true,"content":{"multipart/form-data;boundary=browser":{"schema":{"type":"object","required":["file","note"],"properties":{"file":{},"note":{"type":"string"}},"additionalProperties":false}}}},"responses":{"204":{"description":"accepted"}}}},
                "/events":{"get":{"operationId":"events","responses":{"200":{"description":"events","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}},"additionalProperties":false}}}}}}},
                "/wait":{"get":{"operationId":"wait","responses":{"200":{"description":"cancelled stream","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}},"additionalProperties":false}}}}}}},
                "/cookie":{"get":{"operationId":"cookie","security":[{"session":[]}],"responses":{"204":{"description":"cookie"}}}},
                "/redirect":{"get":{"operationId":"redirect","security":[{"token":[]}],"responses":{"200":{"description":"never followed","content":{"application/json":{"schema":{}}}}}}},
                "/whole-json":{"get":{"operationId":"wholeJson","parameters":whole_query("application/json",json!({"type":"object","properties":{"flag":{"type":"boolean"},"values":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false})),"responses":{"204":{"description":"query"}}}},
                "/whole-text":{"get":{"operationId":"wholeText","parameters":whole_query("text/plain",json!({"type":"string"})),"responses":{"204":{"description":"query"}}}},
                "/whole-form":{"get":{"operationId":"wholeForm","parameters":whole_query("application/x-www-form-urlencoded",json!({"type":"object","required":["foo","bar"],"properties":{"foo":{"type":"string"},"bar":{"type":"boolean"}},"additionalProperties":false})),"responses":{"204":{"description":"query"}}}},
                "/custom":{"additionalOperations":{"SEARCH":{"operationId":"searchCustom","responses":{"204":{"description":"method"}}},"get":{"operationId":"lowerGet","responses":{"204":{"description":"method"}}}}},
                "/styled":{"post":{"operationId":"styled","requestBody":{"required":true,"content":{"multipart/form-data;boundary=browser-style":styled.clone()}},"responses":{"200":{"description":"styled","content":{"multipart/form-data;boundary=browser-style":styled}}}}}
            }
        }),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        "export {};",
        r##"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdtemp} from 'node:fs/promises';
import {dirname,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawn} from 'node:child_process';
import {EventEmitter} from 'node:events';
const dist=dirname(fileURLToPath(import.meta.resolve('@suspect-fixtures/typescript-protocol/operations')));
const counts={number:0,upload:0,parts:0,events:0,wait:0,cancelled:0,cookie:0,leak:0,wholeJson:0,wholeText:0,wholeForm:0,search:0,lowerGet:0,styled:0};
const evidence=new EventEmitter();
const html=String.raw`<!doctype html><html><head><meta charset="utf-8"><title>Native SDK protocol</title></head><body data-sdk-status="pending"><pre id="result"></pre><script type="module">
import {createClient,isSdkError} from '/sdk/operations.js';
const check=(condition,label)=>{if(!condition)throw new Error(label);};
try{
    const client=createClient({auth:{token:'browser-fixture',session:'cookie-fixture'},server:{documentURL:location.href}});
    const number=await client.readNumber();check(number.data.amount.toString()==='9007199254740993.25','exact decimal');check(number.typedHeaders['X-Count']===42,'typed header');
    const bytes=await client.upload({body:new Uint8Array([0,255,128,13,10])});check([...bytes.data].join(',')==='255,0','raw octets');
    await client.parts({body:{file:new Uint8Array([0,254]),note:'snow 雪'}});
    await client.wholeJson({queryValue:{values:[1n,2n],flag:true}});await client.wholeText({queryValue:'a=b & 雪%2F'});await client.wholeForm({queryValue:{foo:'a + b',bar:true}});
    await client.searchCustom();let methodRefused=false;try{await client.lowerGet();}catch(error){methodRefused=isSdkError(error)&&error.kind==='request-representation';}check(methodRefused,'exact lowercase method refusal');
    const styled=await client.styled({body:{values:['a,b&c','雪%2F'],map:{code:42n}}});check(styled.data.map.code===7n&&styled.data.values[0]==='received= & 雪','physical style fields');
    const values=[];for await(const event of (await client.events()).data)values.push(event.data);check(values.join('|')==='snow 雪|[DONE]','standard SSE strings');
    for await(const event of (await client.wait()).data){check(event.data==='first','stream first item');break;}
    let refused=false;try{await client.cookie();}catch(error){refused=isSdkError(error)&&error.kind==='request-representation';}check(refused,'browser Cookie refusal');
    let redirect=false;try{await client.redirect();}catch(error){redirect=isSdkError(error)&&error.kind==='transport';}check(redirect,'redirect error');
    const aborted=new AbortController();aborted.abort();let cancelled=false;try{await client.readNumber({}, {signal:aborted.signal});}catch(error){cancelled=isSdkError(error)&&error.kind==='cancelled';}check(cancelled,'caller abort');
    const proof=await (await fetch('/evidence')).json();check(proof.cancelled===1&&proof.cookie===0&&proof.leak===0&&proof.number===1,'network cleanup and credential boundaries');
    document.body.dataset.sdkStatus='passed';document.getElementById('result').textContent=JSON.stringify({agent:navigator.userAgent,...proof});
}catch(error){document.body.dataset.sdkStatus='failed';document.getElementById('result').textContent=String(error.stack||error);}
</script></body></html>`;
const server=createServer(async(request,response)=>{
    try{
        const url=new URL(request.url,'http://fixture');
        if(url.pathname==='/'){response.writeHead(200,{'content-type':'text/html'});response.end(html);return;}
        if(url.pathname.startsWith('/sdk/')&&!url.pathname.includes('..')){response.writeHead(200,{'content-type':'text/javascript'});response.end(await readFile(join(dist,url.pathname.slice(5))));return;}
        if(url.pathname==='/evidence'){if(counts.cancelled===0)await new Promise(resolve=>evidence.once('cancelled',resolve));response.writeHead(200,{'content-type':'application/json'});response.end(JSON.stringify(counts));return;}
        if(url.pathname==='/api/number'){counts.number++;assert.equal(request.headers.authorization,'Bearer browser-fixture');response.writeHead(200,{'content-type':'application/json','x-count':'42'});response.end('{"amount":9007199254740993.25}');return;}
        if(url.pathname==='/api/events'){counts.events++;response.writeHead(200,{'content-type':'text/event-stream'});response.write('data: snow 雪\n\n');setTimeout(()=>response.end('data: [DONE]\n\n'),20);return;}
        if(url.pathname==='/api/wait'){counts.wait++;response.writeHead(200,{'content-type':'text/event-stream'});response.write('data: first\n\n');response.on('close',()=>{counts.cancelled++;evidence.emit('cancelled');});return;}
        if(url.pathname==='/api/cookie'){counts.cookie++;response.writeHead(204);response.end();return;}
        if(url.pathname==='/api/whole-json'){counts.wholeJson++;assert.equal(request.url,'/api/whole-json?%7B%22flag%22%3Atrue%2C%22values%22%3A%5B1%2C2%5D%7D');response.writeHead(204);response.end();return;}
        if(url.pathname==='/api/whole-text'){counts.wholeText++;assert.equal(request.url,'/api/whole-text?a%3Db%20%26%20%E9%9B%AA%252F');response.writeHead(204);response.end();return;}
        if(url.pathname==='/api/whole-form'){counts.wholeForm++;assert.equal(request.url,'/api/whole-form?bar=true&foo=a+%2B+b');response.writeHead(204);response.end();return;}
        if(url.pathname==='/api/custom'){if(request.method==='SEARCH')counts.search++;else counts.lowerGet++;response.writeHead(204);response.end();return;}
        if(url.pathname==='/api/styled'){
            counts.styled++;const chunks=[];for await(const chunk of request)chunks.push(chunk);const bytes=Buffer.concat(chunks);
            const expected='--browser-style\r\nContent-Disposition: form-data; name="code"\r\n\r\n42\r\n--browser-style\r\nContent-Disposition: form-data; name="values"\r\n\r\na,b&c\r\n--browser-style\r\nContent-Disposition: form-data; name="values"\r\n\r\n雪%2F\r\n--browser-style--\r\n';assert.deepEqual(bytes,Buffer.from(expected));
            response.writeHead(200,{'content-type':'multipart/form-data;boundary=browser-style'});response.end('--browser-style\r\nContent-Disposition: form-data; name="values"\r\n\r\nreceived= & 雪\r\n--browser-style\r\nContent-Disposition: form-data; name="code"\r\n\r\n7\r\n--browser-style--\r\n');return;
        }
        if(url.pathname==='/api/redirect'){assert.equal(request.headers.authorization,'Bearer browser-fixture');response.writeHead(302,{location:'/leak'});response.end();return;}
        if(url.pathname==='/leak'){counts.leak++;response.writeHead(200,{'content-type':'application/json'});response.end('{}');return;}
        if(url.pathname==='/api/upload'||url.pathname==='/api/parts'){
            const chunks=[];for await(const chunk of request)chunks.push(chunk);const body=Buffer.concat(chunks);
            if(url.pathname==='/api/upload'){counts.upload++;assert.deepEqual([...body],[0,255,128,13,10]);response.writeHead(200,{'content-type':'application/octet-stream'});response.end(Buffer.from([255,0]));}
            else {counts.parts++;assert.equal(request.headers['content-type'],'multipart/form-data;boundary=browser');const form=await new Response(body,{headers:{'content-type':request.headers['content-type']}}).formData();assert.equal(form.get('note'),'snow 雪');assert.deepEqual([...new Uint8Array(await form.get('file').arrayBuffer())],[0,254]);response.writeHead(204);response.end();}return;
        }
        response.writeHead(404);response.end();
    }catch(error){response.writeHead(500);response.end(String(error.stack));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const profile=await mkdtemp(join(process.cwd(),'browser-profile-'));
const chrome=process.env.SUSPECT_CHROMIUM??'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const pageURL=`http://127.0.0.1:${server.address().port}/`;
const child=spawn(chrome,['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-extensions','--password-store=basic','--use-mock-keychain','--remote-debugging-port=0',`--user-data-dir=${profile}`,pageURL]);
const stopped=new Promise(resolve=>child.on('close',resolve));
let stderr='';const debugging=new Promise((resolve,reject)=>{child.stderr.on('data',chunk=>{stderr+=chunk;const match=/DevTools listening on (ws:\/\/[^\s]+)/.exec(stderr);if(match)resolve(new URL(match[1]));});child.on('error',reject);child.on('close',()=>reject(new Error(stderr)));});
const timeout=setTimeout(()=>child.kill('SIGKILL'),30000);
let socket;
try{
    const address=await debugging;let target;
    for(let attempt=0;attempt<100;attempt++){const targets=await (await fetch(`http://${address.host}/json/list`)).json();target=targets.find(target=>target.type==='page'&&target.url===pageURL);if(target)break;await new Promise(resolve=>setTimeout(resolve,20));}
    assert.ok(target,'Chromium page target');socket=new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});
    const waiting=new Map();let requestId=0;
    socket.addEventListener('message',event=>{const message=JSON.parse(event.data),request=waiting.get(message.id);if(!request)return;waiting.delete(message.id);if(message.error)request.reject(message.error);else request.resolve(message.result);});
    socket.addEventListener('close',()=>{for(const request of waiting.values())request.reject(new Error(stderr));waiting.clear();},{once:true});
    let evaluated;
    for(let attempt=0;attempt<10;attempt++){
        const id=++requestId,result=new Promise((resolve,reject)=>waiting.set(id,{resolve,reject}));
        socket.send(JSON.stringify({id,method:'Runtime.evaluate',params:{awaitPromise:true,returnByValue:true,expression:`new Promise(resolve=>{const check=()=>{const status=document.body?.dataset.sdkStatus;if(status==='passed'||status==='failed'){observer.disconnect();resolve({status,content:document.getElementById('result').textContent});}};const observer=new MutationObserver(check);observer.observe(document,{subtree:true,childList:true,attributes:true});check();})`}}));
        try{evaluated=await result;break;}catch(error){if(error.code!==-32000||!error.message.includes('context'))throw error;await new Promise(resolve=>setTimeout(resolve,20));}
    }
    assert.equal(evaluated?.result?.value?.status,'passed',JSON.stringify(evaluated)+'\n'+JSON.stringify(counts)+'\n'+stderr);
    assert.deepEqual(counts,{number:1,upload:1,parts:1,events:1,wait:1,cancelled:1,cookie:0,leak:0,wholeJson:1,wholeText:1,wholeForm:1,search:1,lowerGet:0,styled:1});console.log('browser-native-protocol-passed',evaluated.result.value.content);
}finally{clearTimeout(timeout);socket?.close();child.kill();await stopped;server.closeAllConnections();await new Promise(resolve=>server.close(resolve));}
"##,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn unverified_and_ambiguous_protocols_are_source_located_refusals() {
    for (document, code, suffix) in [
        (
            json!({"openapi":"3.2.0","info":{"title":"Ambiguous item grouping","version":"1"},"paths":{"/parts":{"post":{"requestBody":{"content":{"multipart/form-data":{"schema":{"type":"object","properties":{"field":{"type":"array","items":{"type":"array","items":{"type":"string"}}}},"additionalProperties":false},"encoding":{"field":{"style":"form","explode":true}}}}},"responses":{"204":{"description":"accepted"}}}}}}),
            "http-typescript-multipart-style-grouping",
            "/encoding/field",
        ),
        (
            json!({"openapi":"3.1.2","info":{"title":"No vendor stream semantics","version":"1"},"paths":{"/events":{"get":{"responses":{"200":{"description":"legacy stream","content":{"text/event-stream":{"schema":{"type":"object"},"x-speakeasy-sse-sentinel":"[DONE]"}}}}}}}}),
            "http-stream-item-schema-required",
            "/schema",
        ),
        (
            json!({"openapi":"3.1.2","info":{"title":"Ambiguous media","version":"1"},"paths":{"/media":{"get":{"responses":{"200":{"description":"ambiguous","content":{"application/json;profile=a":{"schema":{}},"application/json;charset=utf-8":{"schema":{}}}}}}}}}),
            "http-media-type-ambiguous",
            "/content/application~1json;profile=a",
        ),
    ] {
        let contract = contract(document);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let errors = plan_http(contract, &selected, HttpConfig::expanded()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == code
                && error.source.pointer().ends_with(suffix)
                && !error.at.is_empty()),
            "{code}: {errors:?}"
        );
    }
}

#[test]
#[ignore = "requires the tracked OpenRouter description"]
fn real_extra_openrouter_operations_use_native_json_and_explicit_legacy_byte_profile() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let path = Path::new(&root).join("projects/docs/openapi/openapi.yaml");
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
                Some("listProviders" | "downloadContainerFileContent")
            )
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 2);
    let errors = plan_http(contract.clone(), &selected, HttpConfig::expanded()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-binary-legacy-marker")
    );
    let plan = plan_http(
        contract,
        &selected,
        HttpConfig {
            legacy_binary_string: true,
            ..HttpConfig::expanded()
        },
    )
    .unwrap();
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient({auth:{apiKey:'fixture-only'}});
export async function nativeCalls(){const providers=await client.listProviders();const file=await client.downloadContainerFileContent({container_id:'sess_a',file_id:'cfile_a'});const bytes:Uint8Array=file.data;return {providers,bytes};}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient} from './dist/operations.js';
const paths=[];const server=createServer((request,response)=>{paths.push(request.url);assert.equal(request.headers.authorization,'Bearer fixture-only');if(request.url==='/api/v1/providers'){response.writeHead(200,{'content-type':'application/json'});response.end('{"data":[]}');}else{response.writeHead(200,{'content-type':'application/octet-stream'});response.end(Buffer.from([0,255,1]));}});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{const client=createClient({auth:{apiKey:'fixture-only'},serverURL:`http://127.0.0.1:${server.address().port}/api/v1`});assert.deepEqual((await client.listProviders()).data.data,[]);assert.deepEqual([...(await client.downloadContainerFileContent({container_id:'sess_a',file_id:'cfile_a'})).data],[0,255,1]);assert.deepEqual(paths,['/api/v1/providers','/api/v1/containers/sess_a/files/cfile_a/content']);}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn large_source_cardinalities_remain_exact_while_transport_allocations_stay_bounded() {
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Exact resource metadata","version":"1"},"paths":{
            "/bytes":{"post":{"operationId":"sendBytes","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"maxLength":u64::MAX}}}},"responses":{"200":{"description":"bytes","content":{"application/octet-stream":{"schema":{"maxLength":u64::MAX}}}}}}},
            "/parts":{"post":{"operationId":"sendParts","requestBody":{"required":true,"content":{"multipart/form-data;boundary=exact":{"schema":{"type":"object","maxProperties":u64::MAX,"required":["note"],"properties":{"note":{"type":"string"},"tags":{"type":"array","minItems":1,"maxItems":u64::MAX,"items":{"type":"string"}}},"additionalProperties":false}}}},"responses":{"204":{"description":"accepted"}}}}
        }}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        "export {};",
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,operationMetadata} from './dist/operations.js';
const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{if(request.url==='/bytes'){assert.deepEqual([...Buffer.concat(chunks)],[0,255]);response.writeHead(200,{'content-type':'application/octet-stream'});response.end(Buffer.from([255,0]));}else{response.writeHead(204);response.end();}});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});assert.deepEqual([...(await client.sendBytes({body:new Uint8Array([0,255])})).data],[255,0]);await client.sendParts({body:{note:'native',tags:['one']}});assert.equal(operationMetadata.sendBytes.body.media[0].representation.bytes.declared_max_bytes.value.toString(),'18446744073709551615');}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn custom_method_tokens_are_exact_and_fetch_normalization_is_an_explicit_refusal() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut document = fixture["oas32Methods"]["document"].clone();
    document["paths"]["/methods"]["additionalOperations"]["RESET"] = json!({"operationId":"reset","responses":{"205":{"description":"reset","content":{"application/json":{"schema":{"not":{}}}}}}});
    let plan = plan(document, HttpConfig::expanded());
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function methods(){const response=await client.lowerHead();const value=response.data.seen;const reset=await client.reset();const absent:undefined=reset.data;return {value,absent};}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer,connect} from 'node:net';
import {createClient,isSdkError} from './dist/operations.js';
const lines=[];
const server=createServer(socket=>{let raw='';socket.on('data',bytes=>{raw+=bytes.toString('latin1');if(!raw.includes('\r\n\r\n'))return;const line=raw.split('\r\n')[0];lines.push(line);const method=line.split(' ')[0];const body=method==='head'?'{"seen":true}':method==='RESET'?'':'ok';const status=method==='RESET'?205:200;socket.end(`HTTP/1.1 ${status} Fixture\r\nContent-Type: ${method==='head'?'application/json':'text/plain'}\r\nContent-Length: ${Buffer.byteLength(body)}\r\nConnection: close\r\n\r\n${body}`);});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const serverURL=`http://127.0.0.1:${server.address().port}`;
const rawFetch=(input,init)=>new Promise((resolve,reject)=>{const url=new URL(input),socket=connect({host:url.hostname,port:Number(url.port)}),chunks=[];socket.on('error',reject);socket.on('connect',()=>socket.write(`${init.method} ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nConnection: close\r\n\r\n`));socket.on('data',chunk=>chunks.push(chunk));socket.on('end',()=>{const bytes=Buffer.concat(chunks),split=bytes.indexOf('\r\n\r\n'),text=bytes.subarray(0,split).toString(),status=Number(text.split(' ')[1]);resolve(new Response(status===205?null:bytes.subarray(split+4),{status,headers:{'content-type':/Content-Type: ([^\r]+)/.exec(text)[1]}}));});});
try{
    const client=createClient({serverURL}),explicit=createClient({serverURL,fetch:rawFetch});
    for(const [name,token] of [['copy','COPY'],['extensionNamedMethod','x-PING'],['fixedQuery','QUERY'],['fixedGet','GET']]){await client[name]();assert.equal(lines.at(-1),`${token} /methods HTTP/1.1`);}
    for(const [name,token] of [['mixedGet','GeT'],['lowerGet','get'],['lowerHead','head']]){
        const before=lines.length;await assert.rejects(client[name](),error=>isSdkError(error)&&error.kind==='request-representation');assert.equal(lines.length,before);
        const response=await explicit[name]();assert.equal(lines.at(-1),`${token} /methods HTTP/1.1`);if(token==='head')assert.equal(response.data.seen,true,'lowercase head is not HTTP HEAD');
    }
    assert.equal((await client.reset()).data,undefined);
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn whole_query_json_text_and_form_have_no_name_prefix_or_double_encoding() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut document =
        json!({"openapi":"3.2.0","info":{"title":"Whole-query vectors","version":"1"},"paths":{}});
    for (index, case) in fixtures["querystringCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        document["paths"][format!("/q{index}")] = json!({"get":{"operationId":format!("wholeQuery{index}"),"parameters":[case["parameter"].clone()],"responses":{"200":{"description":"query","content":{"text/plain":{"schema":{"type":"string"}}}}}}});
    }
    let plan = plan(document, HttpConfig::expanded());
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function query(){
    await client.wholeQuery0({completeQuery:'a=b & 雪%2F'});
    await client.wholeQuery1({ignoredName:{numbers:[1n,2n],flag:null}});
    await client.wholeQuery2({alsoUnused:{foo:'a + b',bar:true,items:[1n,2n]}});
    // @ts-expect-error whole text is required, and no empty input is invented
    await client.wholeQuery0();
    // @ts-expect-error whole form retains its actual required model fields
    await client.wholeQuery2({alsoUnused:{foo:'text'}});
    // @ts-expect-error integer fields retain their native bigint type
    await client.wholeQuery2({alsoUnused:{foo:'text',bar:true,items:['bad']}});
}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const paths=[];const server=createServer((request,response)=>{paths.push(request.url);response.writeHead(200,{'content-type':'text/plain'});response.end('ok');});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
    await client.wholeQuery0({completeQuery:'a=b & 雪%2F'});
    await client.wholeQuery1({ignoredName:{numbers:[1n,2n],flag:null}});
    await client.wholeQuery2({alsoUnused:{foo:'a + b',bar:true,items:[1n,2n]}});
    assert.deepEqual(paths,['/q0?a%3Db%20%26%20%E9%9B%AA%252F','/q1?%7B%22flag%22%3Anull%2C%22numbers%22%3A%5B1%2C2%5D%7D','/q2?bar=true&foo=a+%2B+b&items=1&items=2']);
    await client.wholeQuery1();await client.wholeQuery2();assert.deepEqual(paths.slice(3),['/q1','/q2']);
    const before=paths.length;
    await assert.rejects(client.wholeQuery0({}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.wholeQuery2({alsoUnused:{foo:'x',bar:true,items:['bad']}}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.wholeQuery2({alsoUnused:{foo:'x',bar:true,extra:'bad'}}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(createClient({serverURL,maxRequestBytes:4}).wholeQuery0({completeQuery:'abcdef'}),error=>isSdkError(error)&&error.kind==='resource-limit');
    assert.equal(paths.length,before);
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn rfc6570_multipart_expands_physical_fields_without_uri_encoding_or_query_text() {
    let plan = plan(
        json!({"openapi":"3.1.2","info":{"title":"Physical multipart style vectors","version":"1"},"paths":{"/style":{"post":{"operationId":"styledUpload","requestBody":{"required":true,"content":{"multipart/form-data;boundary=style-bound":{
        "schema":{"type":"object","required":["colors","csv","filter","note","pipes","rgb","spaces"],"additionalProperties":false,"properties":{
            "colors":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"string"}},
            "csv":{"type":"array","items":{"type":"integer"}},
            "extras":{"type":"object","additionalProperties":{"type":"string"}},
            "filter":{"type":"object","properties":{"snow 雪":{"type":"string"}},"required":["snow 雪"],"additionalProperties":false},
            "note":{"type":"string"},"pipes":{"type":"array","items":{"type":"string"}},
            "rgb":{"type":"object","properties":{"B":{"type":"integer"},"G":{"type":"integer"},"R":{"type":"integer"}},"required":["B","G","R"],"additionalProperties":false},
            "spaces":{"type":"array","items":{"type":"string"}}
        }},"encoding":{
            "colors":{"style":"form","explode":true},"csv":{"style":"form","explode":false},"extras":{"style":"form","explode":true},
            "filter":{"style":"deepObject","explode":true},"note":{"style":"form","allowReserved":true,"contentType":"application/json","headers":{"X-Note":{"required":true,"schema":{"type":"boolean"}}}},
            "pipes":{"style":"pipeDelimited"},"rgb":{"style":"form","explode":true},"spaces":{"style":"spaceDelimited"}
        }
    }}},"responses":{"204":{"description":"accepted"}}}}}}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
const client=createClient();
export async function typedParts(){await client.styledUpload({body:{colors:['a,b & c','snow 雪%2F'],csv:[1n,9007199254740993n],filter:{'snow 雪':'[brackets]=&?'},note:{data:'a=b & 雪%2F\r\nline',headers:{'X-Note':true}},pipes:['a','b'],rgb:{B:150n,G:200n,R:100n},spaces:['one','two']}});}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const received=[];const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{received.push(Buffer.concat(chunks));response.writeHead(204);response.end();});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const body={colors:['a,b & c','snow 雪%2F'],csv:[1n,9007199254740993n],filter:{'snow 雪':'[brackets]=&?'},note:{data:'a=b & 雪%2F\r\nline',headers:{'X-Note':true}},pipes:['a','b'],rgb:{B:150n,G:200n,R:100n},spaces:['one','two']};
const expected='--style-bound\r\nContent-Disposition: form-data; name="colors"\r\n\r\na,b & c\r\n--style-bound\r\nContent-Disposition: form-data; name="colors"\r\n\r\nsnow 雪%2F\r\n--style-bound\r\nContent-Disposition: form-data; name="csv"\r\n\r\n1,9007199254740993\r\n--style-bound\r\nContent-Disposition: form-data; name="filter[snow 雪]"\r\n\r\n[brackets]=&?\r\n--style-bound\r\nX-Note: true\r\nContent-Disposition: form-data; name="note"\r\n\r\na=b & 雪%2F\r\nline\r\n--style-bound\r\nContent-Disposition: form-data; name="pipes"\r\n\r\na|b\r\n--style-bound\r\nContent-Disposition: form-data; name="B"\r\n\r\n150\r\n--style-bound\r\nContent-Disposition: form-data; name="G"\r\n\r\n200\r\n--style-bound\r\nContent-Disposition: form-data; name="R"\r\n\r\n100\r\n--style-bound\r\nContent-Disposition: form-data; name="spaces"\r\n\r\none two\r\n--style-bound--\r\n';
try{
    const serverURL=`http://127.0.0.1:${server.address().port}`,client=createClient({serverURL});
    await client.styledUpload({body});assert.deepEqual(received[0],Buffer.from(expected));
    const form=await new Response(received[0],{headers:{'content-type':'multipart/form-data;boundary=style-bound'}}).formData();
    assert.deepEqual(form.getAll('colors'),['a,b & c','snow 雪%2F']);assert.equal(form.get('B'),'150');assert.equal(form.get('rgb'),null);assert.equal(form.get('filter[snow 雪]'),'[brackets]=&?');
    const before=received.length;
    for(const value of [{...body,spaces:['two words']},{...body,pipes:['a|b']},{...body,extras:{colors:'collision'}},{...body,note:{data:'x',headers:{}}},{...body,note:{...body.note,contentType:'application/json'}},{...body,note:{...body.note,data:'\r\n--style-bound\r\nInjected'}}])await assert.rejects(client.styledUpload({body:value}),error=>isSdkError(error)&&error.kind==='request-representation');
    await assert.rejects(client.styledUpload({body:{...body,colors:[]}}),error=>isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(createClient({serverURL,maxPartBytes:2}).styledUpload({body}),error=>isSdkError(error)&&error.kind==='resource-limit');
    assert.equal(received.length,before);
}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn rfc6570_multipart_responses_reconstruct_typed_items_and_enforce_group_metadata() {
    let media = json!({"schema":{"type":"object","required":["colors","csv","filter","note","rgb"],"additionalProperties":false,"properties":{
        "colors":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string"}},
        "csv":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"array","items":{"type":"string"}}},
        "filter":{"type":"object","properties":{"snow 雪":{"type":"string"}},"required":["snow 雪"],"additionalProperties":false},
        "note":{"type":"string"},"rgb":{"type":"object","required":["B","G","R"],"properties":{"B":{"type":"integer"},"G":{"type":"integer"},"R":{"type":"integer"}},"additionalProperties":false}
    }},"encoding":{"colors":{"style":"form","explode":true},"csv":{"style":"form","explode":false},"filter":{"style":"deepObject","explode":true},"note":{"style":"form","headers":{"X-Note":{"required":true,"schema":{"type":"boolean"}}}},"rgb":{"style":"form","explode":true,"headers":{"X-Group":{"required":true,"schema":{"type":"string"}}}}}});
    let plan = plan(
        json!({"openapi":"3.2.0","info":{"title":"Physical MIME response mapping","version":"1"},"paths":{"/style":{"get":{"operationId":"styledDownload","parameters":[{"name":"mode","in":"query","schema":{"type":"string"}}],"responses":{"200":{"description":"styled","content":{"multipart/form-data;boundary=reply":media}}}}}}}),
        HttpConfig::expanded(),
    );
    native(
        &plan,
        r#"
import {createClient} from './operations.js';
export async function typedResponse(){const result=await createClient().styledDownload();const b:bigint=result.data.rgb.data.B;const note:string=result.data.note.data;const group:string=result.data.rgb.headers['X-Group'];const item:readonly string[]|undefined=result.data.csv[0];return {b,note,group,item};}
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const literal='--reply\r\nContent-Disposition: form-data; name="colors"\r\n\r\na,b&c\r\n--reply\r\nContent-Disposition: form-data; name="colors"\r\n\r\n雪%2F\r\n--reply\r\nContent-Disposition: form-data; name="csv"\r\n\r\na,b\r\n--reply\r\nContent-Disposition: form-data; name="csv"\r\n\r\nc,d\r\n--reply\r\nContent-Disposition: form-data; name="filter[snow 雪]"\r\n\r\n[brackets]=&?\r\n--reply\r\nContent-Disposition: form-data; name="note"\r\nX-Note: true\r\n\r\nline1\r\nline2\r\n--reply\r\nContent-Disposition: form-data; name="R"\r\nX-Group: stable\r\n\r\n100\r\n--reply\r\nContent-Disposition: form-data; name="B"\r\nX-Group: stable\r\n\r\n150\r\n--reply\r\nContent-Disposition: form-data; name="G"\r\nX-Group: stable\r\n\r\n200\r\n--reply--\r\n';
const server=createServer((request,response)=>{const mode=new URL(request.url,'http://fixture').searchParams.get('mode');let body=literal;if(mode==='header')body=body.replace('X-Note: true\r\n','');if(mode==='group')body=body.replace('X-Group: stable','X-Group: changed');if(mode==='duplicate')body=body.replace('name="G"','name="B"');if(mode==='item')body=body.replace('\r\n150\r\n','\r\nnot-an-integer\r\n');if(mode==='bracket')body=body.replace('filter[snow 雪]','filter[snow][nested]');response.writeHead(200,{'content-type':'multipart/form-data;boundary=reply'});response.end(body);});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});const response=await client.styledDownload();assert.deepEqual(response.data.colors,['a,b&c','雪%2F']);assert.deepEqual(response.data.csv,[['a','b'],['c','d']]);assert.equal(response.data.filter['snow 雪'],'[brackets]=&?');assert.equal(response.data.note.data,'line1\r\nline2');assert.equal(response.data.note.headers['X-Note'],true);assert.equal(response.data.rgb.data.B,150n);assert.equal(response.data.rgb.data.G,200n);assert.equal(response.data.rgb.headers['X-Group'],'stable');for(const mode of ['header','group','duplicate','item','bracket'])await assert.rejects(client.styledDownload({mode}),error=>isSdkError(error)&&error.kind==='response-decoding');}finally{await new Promise(resolve=>server.close(resolve));}
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn whole_query_conflicts_and_custom_method_errors_remain_source_located() {
    let query = json!({"name":"whole","in":"querystring","content":{"text/plain":{"schema":{"type":"string"}}}});
    let base = json!({"openapi":"3.2.0","info":{"title":"Whole-query controls","version":"1"},"paths":{"/search":{"get":{"operationId":"search","parameters":[query.clone()],"responses":{"204":{"description":"ok"}}}}}});
    for (case, code) in [
        ("ordinary", "http-querystring-query-conflict"),
        ("duplicate", "http-querystring-duplicate"),
        ("credential", "http-querystring-security-conflict"),
        ("fixed-method", "http-additional-method-fixed"),
    ] {
        let mut document = base.clone();
        match case {
            "ordinary" => document["paths"]["/search"]["get"]["parameters"]
                .as_array_mut()
                .unwrap()
                .push(json!({"name":"q","in":"query","schema":{"type":"string"}})),
            "duplicate" => {
                let mut other = query.clone();
                other["name"] = json!("other");
                document["paths"]["/search"]["get"]["parameters"]
                    .as_array_mut()
                    .unwrap()
                    .push(other);
            }
            "credential" => {
                document["components"] = json!({"securitySchemes":{"queryKey":{"type":"apiKey","in":"query","name":"key"}}});
                document["security"] = json!([{"queryKey":[]}]);
            }
            "fixed-method" => {
                document["paths"]["/search"]["additionalOperations"] = json!({"GET":{"responses":{"204":{"description":"invalid duplicate fixed token"}}}})
            }
            _ => unreachable!(),
        }
        let contract = contract(document);
        let selected = contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>();
        let errors = plan_http(contract, &selected, HttpConfig::expanded()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == code && !error.at.is_empty()),
            "{case}: {errors:?}"
        );
    }
}

fn contract(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn plan(document: Value, config: HttpConfig) -> HttpPlan {
    let contract = contract(document);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_http(contract, &selected, config).unwrap()
}

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().expect("required native SDK tool");
    assert!(
        output.status.success(),
        "native fixture {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn native(plan: &HttpPlan, types: &str, consumer: &str) {
    native_impl(plan, types, consumer, false);
}

fn native_impl(plan: &HttpPlan, types: &str, consumer: &str, execute_types: bool) {
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(
        &emit_http(
            plan,
            &PackageConfig {
                name: "@suspect-fixtures/typescript-protocol".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        directory.path(),
    )
    .unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), types).unwrap();
    checked(
        Command::new("tsc").current_dir(&root).args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--declaration",
            "--outDir",
            "dist",
            "--pretty",
            "false",
            "source/index.ts",
            "examples/validated.ts",
            "examples/first-request.ts",
            "consumer.ts",
        ]),
        &root,
    );
    std::fs::write(root.join("consumer.mjs"), consumer).unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    checked(
        Command::new(&node)
            .current_dir(&root)
            .arg("dist/examples/validated.js"),
        &root,
    );
    if execute_types {
        checked(
            Command::new(&node)
                .current_dir(&root)
                .arg("dist/consumer.js"),
            &root,
        );
    }
    checked(
        Command::new(&node).current_dir(&root).arg("consumer.mjs"),
        &root,
    );
    if let Some(node) = std::env::var_os("SUSPECT_NODE24_BIN") {
        checked(
            Command::new(&node)
                .current_dir(&root)
                .arg("dist/examples/validated.js"),
            &root,
        );
        if execute_types {
            checked(
                Command::new(&node)
                    .current_dir(&root)
                    .arg("dist/consumer.js"),
                &root,
            );
        }
        checked(
            Command::new(node).current_dir(&root).arg("consumer.mjs"),
            &root,
        );
    }
    if std::env::var_os("SUSPECT_PROTOCOL_INSTALL").is_some() {
        installed(
            &node,
            &root,
            directory.path(),
            types,
            consumer,
            execute_types,
        );
    }
}

fn installed(
    node: &std::ffi::OsStr,
    package: &Path,
    root: &Path,
    types: &str,
    consumer: &str,
    execute_types: bool,
) {
    let executable = checked(
        Command::new(node).args(["--print", "process.execPath"]),
        root,
    );
    let executable = std::path::PathBuf::from(String::from_utf8(executable.stdout).unwrap().trim());
    let npm = executable
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let npm_command = |cwd: &Path| {
        let mut command = Command::new(&executable);
        command.arg(&npm).current_dir(cwd);
        command.env(
            "PATH",
            std::env::join_paths(
                std::iter::once(executable.parent().unwrap().to_owned()).chain(
                    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
                ),
            )
            .unwrap(),
        );
        command
    };
    checked(
        npm_command(package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        root,
    );
    checked(npm_command(package).args(["run", "build"]), root);
    for generated_test in ["dist/consumer.js", "dist/consumer.d.ts"] {
        std::fs::remove_file(package.join(generated_test)).unwrap();
    }
    let packed = checked(
        npm_command(package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
        root,
    );
    let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
    let installed = root.join("installed");
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
            .arg(package.join(packed[0]["filename"].as_str().unwrap())),
        root,
    );
    let package_name = "@suspect-fixtures/typescript-protocol";
    let types = types
        .replace("'./operations.js'", &format!("'{package_name}/operations'"))
        .replace("'./json.js'", &format!("'{package_name}/json'"))
        .replace("'./source/index.js'", &format!("'{package_name}'"));
    let consumer = consumer
        .replace(
            "'./dist/operations.js'",
            &format!("'{package_name}/operations'"),
        )
        .replace("'./dist/json.js'", &format!("'{package_name}/json'"));
    std::fs::write(installed.join("consumer.ts"), types).unwrap();
    std::fs::write(installed.join("consumer.mjs"), consumer).unwrap();
    let floor = root.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-floor");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(tools.join(file), floor.join(file)).unwrap();
    }
    checked(
        npm_command(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        root,
    );
    for compiler in [
        floor.join("node_modules/typescript/bin/tsc"),
        package.join("node_modules/typescript/bin/tsc"),
    ] {
        let version = checked(
            Command::new(&executable).arg(&compiler).arg("--version"),
            root,
        );
        checked(
            Command::new(&executable)
                .arg(compiler)
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
            root,
        );
        println!(
            "installed declarations: {}",
            String::from_utf8_lossy(&version.stdout).trim()
        );
    }
    for node in
        std::iter::once(executable.into_os_string()).chain(std::env::var_os("SUSPECT_NODE24_BIN"))
    {
        checked(
            Command::new(&node)
                .current_dir(&installed)
                .arg("consumer.mjs"),
            root,
        );
        if execute_types {
            checked(
                Command::new(&node)
                    .arg(package.join("node_modules/typescript/bin/tsc"))
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
                        "--outDir",
                        "typed",
                        "consumer.ts",
                    ]),
                root,
            );
            checked(
                Command::new(&node)
                    .current_dir(&installed)
                    .arg("typed/consumer.js"),
                root,
            );
        }
        let version = checked(Command::new(node).arg("--version"), root);
        println!(
            "installed ESM consumer: {}",
            String::from_utf8_lossy(&version.stdout).trim()
        );
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn original_m2_fixture_ts_and_js_consumers_preserve_the_existing_public_api() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let wanted = ["createWidget", "listWidgets", "getWidget", "updateWidget"];
    let selected = contract
        .operations()
        .filter(|operation| {
            operation
                .operation_id()
                .is_some_and(|id| wanted.contains(&id))
        })
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    native_impl(
        &plan,
        &include_str!("fixtures/m2/ts_consumer.ts").replace("__PACKAGE__", "./source/index.js"),
        &include_str!("fixtures/m2/js_consumer.mjs")
            .replace("__PACKAGE__", "@suspect-fixtures/typescript-protocol"),
        true,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn verified_baseline_preserves_native_calls_and_corrects_case_insensitive_wire_metadata() {
    let plan = plan(
        json!({
            "openapi":"3.1.2", "info":{"title":"Native protocol projection","version":"1"},
            "servers":[{"url":"https://example.test/v1"}], "security":[{"token":[]}],
            "components":{"securitySchemes":{"token":{"type":"http","scheme":"BeArEr"}}},
            "paths":{"/credits":{"get":{"operationId":"getCredits","responses":{
                "200":{"description":"exact decimals", "content":{"Application/JSON":{"schema":{
                    "type":"object", "required":["amount"], "properties":{"amount":{"type":"number"}}, "additionalProperties":false
                }}}}
            }}}}
        }),
        HttpConfig::default(),
    );
    native(
        &plan,
        r#"
import {createClient, getCredits, type GetCreditsSuccess} from './operations.js';
import {JsonNumber} from './json.js';
const client = createClient({auth:{token:'fixture-only'}});
export async function call(): Promise<JsonNumber> {
    const response: GetCreditsSuccess = await client.getCredits();
    await getCredits({auth:{token:'fixture-only'}});
    return response.data.amount;
}
// @ts-expect-error the source-named credential is still required
createClient({auth:{}});
"#,
        r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient, isSdkError} from './dist/operations.js';
let calls=0;
const server=createServer((request,response)=>{
    calls++;
    assert.equal(request.url,'/v1/credits');
    assert.equal(request.headers.authorization,'Bearer fixture-only');
    response.writeHead(200,{'Content-Type':'Application/JSON; charset="UTF-8"'});
    response.end('{"amount":9007199254740993.0000000001}');
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
    const client=createClient({auth:{token:'fixture-only'},serverURL:`http://127.0.0.1:${server.address().port}/v1`});
    const response=await client.getCredits();
    assert.equal(response.data.amount.toString(),'9007199254740993.0000000001');
    assert.equal(response.contentType,'application/json');
    const abort=new AbortController(); abort.abort();
    await assert.rejects(client.getCredits({}, {signal:abort.signal}),e=>isSdkError(e)&&e.kind==='cancelled');
    assert.equal(calls,1);
} finally { await new Promise(resolve=>server.close(resolve)); }
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn literal_parameter_vectors_reach_native_fetch_with_styles_utf8_headers_and_cookies() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let cases = fixture["parameterCases"].as_array().unwrap();
    let mut document = json!({"openapi":"3.2.0","info":{"title":"Native parameter vectors","version":"1"},"paths":{}});
    for (index, case) in cases.iter().enumerate() {
        let path = if case["parameter"]["in"] == "path" {
            format!("/vectors/{index}/{{color}}")
        } else {
            format!("/vectors/{index}")
        };
        document["paths"][&path] = json!({"get":{"operationId":format!("parameter{index}"),"parameters":[case["parameter"].clone()],"responses":{"200":{"description":"recorded"}}}});
    }
    document["paths"]["/guards"] = json!({"get":{"operationId":"guards","parameters":[
        {"name":"reserved","in":"query","allowReserved":true,"schema":{"type":"string"}},
        {"name":"object","in":"query","schema":{"type":"object"}},
        {"name":"space","in":"query","style":"spaceDelimited","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"X-Value","in":"header","schema":{"type":"string"}}
    ],"responses":{"200":{"description":"recorded"}}}});
    let plan = plan(document, HttpConfig::expanded());
    let guard_members = plan
        .operations()
        .iter()
        .find(|operation| operation.operation_id == "guards")
        .unwrap()
        .parameters
        .iter()
        .map(|parameter| (parameter.wire_name.clone(), json!(parameter.native_name)))
        .collect::<serde_json::Map<_, _>>();
    let vectors = cases.iter().enumerate().map(|(index,case)| {
        let name=format!("parameter{index}");
        let operation=plan.operations().iter().find(|op|op.operation_id==name).unwrap();
        json!({"operation":name,"member":operation.parameters[0].native_name,"wireName":case["parameter"]["name"],"location":case["parameter"]["in"],"value":case["value"],"wire":case["wire"],"path":format!("/vectors/{index}")})
    }).collect::<Vec<_>>();
    native(&plan, r#"
import {createClient} from './operations.js';
const client=createClient();
export async function typedInputs() {
    await client.parameter2({color:{R:100n,G:200n,B:150n}});
    await client.parameter1({color:['blue','black']});
    await client.parameter26({color:['blue','black']});
    // @ts-expect-error: unbounded integer inputs keep their bigint model type
    await client.parameter2({color:{R:100}});
    // @ts-expect-error: path value is required
    await client.parameter0();
    // @ts-expect-error: arrays cannot contain nested objects
    await client.parameter1({color:[{}]});
}
"#, &r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,isSdkError} from './dist/operations.js';
const vectors=__VECTORS__;
const guardMembers=__GUARDS__;
let received;
let calls=0;
const server=createServer((request,response)=>{calls++;received={url:request.url,headers:request.headers};response.writeHead(200);response.end(Buffer.from([0,255,1]));});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try {
    const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});
    for (const vector of vectors) {
        const response=await client[vector.operation]({[vector.member]:vector.value});
        assert.deepEqual([...response.data],[0,255,1]);
        if(vector.location==='path') assert.equal(received.url,vector.path+'/'+vector.wire);
        if(vector.location==='query') assert.equal(received.url,vector.path+'?'+vector.wire);
        if(vector.location==='header') assert.equal(received.headers[vector.wireName.toLowerCase()],vector.wire);
        if(vector.location==='cookie') assert.equal(received.headers.cookie,vector.wire);
    }
    const before=calls;
    for(const input of [{reserved:'a&admin=true'},{reserved:'a+b'},{reserved:"a'b"},{object:{nested:{x:1}}},{space:['two words']},{'X-Value':'ok\r\nX-Injected: true'}]) {
        await assert.rejects(client.guards(Object.fromEntries(Object.entries(input).map(([key,value])=>[guardMembers[key],value]))),error=>isSdkError(error)&&error.kind==='request-representation');
    }
    await assert.rejects(client.parameter0({color:'..'}),error=>isSdkError(error)&&error.kind==='request-representation');
    await assert.rejects(client.parameter1({color:[]}),error=>isSdkError(error)&&error.kind==='request-representation');
    const accessor={};Object.defineProperty(accessor,'reserved',{enumerable:true,get(){throw new Error('getter executed');}});
    await assert.rejects(client.guards(accessor),error=>isSdkError(error)&&error.kind==='request-validation');
    assert.equal(calls,before,'invalid parameter inputs never reach Fetch');
} finally {await new Promise(resolve=>server.close(resolve));}
"#.replace("__VECTORS__",&serde_json::to_string(&vectors).unwrap()).replace("__GUARDS__",&Value::Object(guard_members).to_string()));
}
