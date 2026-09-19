//! Independent native checks for the generated fetch runtime.

use std::process::Command;

#[test]
#[ignore = "requires pinned native TypeScript and Node.js"]
fn fetch_runtime_preserves_wire_contract_and_failure_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("typescript");
    std::fs::create_dir_all(&root).unwrap();
    for (name, content) in [
        (
            "runtime.ts",
            include_str!("../src/typescript/http/runtime.ts"),
        ),
        ("codecs.ts", include_str!("../src/typescript/codecs.ts")),
        ("json.ts", include_str!("../src/typescript/json.ts")),
        (
            "validation.ts",
            include_str!("../src/typescript/validation.ts"),
        ),
        ("pattern.ts", include_str!("../src/typescript/pattern.ts")),
    ] {
        std::fs::write(root.join(name), content).unwrap();
    }
    std::fs::write(root.join("consumer.ts"), CONSUMER).unwrap();
    let compiled = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "commonjs",
            "--lib",
            "ES2022,DOM,DOM.Iterable",
            "--pretty",
            "false",
            "--outDir",
            "dist",
            "consumer.ts",
        ])
        .output()
        .expect("native TypeScript required");
    assert!(
        compiled.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiled.stdout),
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = Command::new("node")
        .current_dir(&root)
        .arg("dist/consumer.js")
        .output()
        .expect("native Node required");
    assert!(
        executed.status.success(),
        "{}{}",
        String::from_utf8_lossy(&executed.stdout),
        String::from_utf8_lossy(&executed.stderr)
    );
}

const CONSUMER: &str = r#"
import { executeOperation, isDeclaredApiError, isSdkError, type ApiResponse, type DeclaredApiError, type OperationDescriptor, type ReadonlyResponseData } from './runtime.js';
import { ModelCodecError, type ModelCodec } from './codecs.js';
declare function require(name: string): any;
declare const Buffer: any;
const assert = require('node:assert/strict');
const http = require('node:http');
const source = (pointer: string) => ({ document: 'file:///openrouter.yaml', pointer });
type MutableBody = {nested:{value:string};items:string[]};
function responseTypeChecks(error: DeclaredApiError<MutableBody,429,'application/json'>, success: ApiResponse<MutableBody,200,'application/json'>): ReadonlyResponseData<MutableBody> {
  // @ts-expect-error: declared error bodies are recursively immutable
  error.response.data.nested.value='changed';
  // @ts-expect-error: arrays in declared error bodies cannot be pushed
  error.response.data.items.push('changed');
  success.data.nested.value='allowed';
  success.data.items.push('allowed');
  return error.response.data;
}
const jsonCodec = <T>(check: (value: unknown) => boolean = () => true): ModelCodec<T> => ({
  encode(value: T): string { if (!check(value)) throw new ModelCodecError('invalid','invalid'); return JSON.stringify(value); },
  decode(text: string): T { const value: unknown = JSON.parse(text); if (!check(value)) throw new ModelCodecError('invalid','invalid'); return value as T; },
});
const exactBody: ModelCodec<{limit: unknown}> = {
  encode(value) { if (typeof value !== 'object' || value === null) throw new Error('invalid'); return '{"limit":1.0000000000000000001}'; },
  decode(text) { return JSON.parse(text) as {limit: unknown}; },
};
type Input = { path: { hash: string }, body: { limit: unknown } };
type Success = {status:200,contentType:'application/json',headers:Headers,data:{ok:boolean}};
const operation = source('/paths/~1keys~1{hash}/patch');
const errorResponse = source('/paths/~1keys~1{hash}/patch/responses/429');
const descriptor: OperationDescriptor<Input, Success, unknown> = {
  operationId:'updateKeys', source:operation, method:'PATCH', pathTemplate:'/keys/{hash}', serverURL:'https://openrouter.ai/api/v1',
  security:{kind:'httpBearer',schemeName:'apiKey',source:source('/components/securitySchemes/apiKey')},
  inputMembers:['path','body'],
  parameters:[{name:'hash',location:'path',required:true,explode:false,array:false,source:source('/paths/~1keys~1{hash}/patch/parameters/0'),codec:jsonCodec<string>(v=>typeof v==='string'&&v.length>0),read:i=>i.path.hash}],
  body:{source:source('/paths/~1keys~1{hash}/patch/requestBody'),required:true,contentType:'application/json',codec:exactBody,read:i=>i.body},
  responses:[
    {status:200,contentType:'application/json',source:source('/paths/~1keys~1{hash}/patch/responses/200'),success:true,codec:jsonCodec(v=>typeof v==='object'&&v!==null&&'ok' in v)},
    {status:201,contentType:'application/json',source:source('/paths/~1keys/post/responses/201'),success:true,codec:jsonCodec(v=>typeof v==='object'&&v!==null&&'ok' in v)},
    ...[400,401,403,404,500].map(status=>({status,contentType:'application/json' as const,source:source(`/paths/~1keys~1{hash}/patch/responses/${status}`),success:false,codec:jsonCodec(v=>typeof v==='object'&&v!==null&&'error' in v)})),
    {status:429,contentType:'application/json',source:errorResponse,success:false,codec:jsonCodec(v=>typeof v==='object'&&v!==null&&'error' in v)},
  ],
};

async function main(): Promise<void> {
  const requests: {url:string,method:string,authorization:string|null,accept:string|null,contentType:string|null,body:string}[] = [];
  let status = 200;
  let responseBody = '{"ok":true}';
  const server = http.createServer((request:any,response:any) => {
    const chunks:any[]=[]; request.on('data',(chunk:any)=>chunks.push(chunk)); request.on('end',()=>{
      requests.push({url:request.url,method:request.method,authorization:request.headers.authorization??null,accept:request.headers.accept??null,contentType:request.headers['content-type']??null,body:Buffer.concat(chunks).toString('utf8')});
      response.writeHead(status,{'content-type':'application/json; charset=utf-8'}); response.end(responseBody);
    });
  });
  await new Promise<void>(resolve=>server.listen(0,'127.0.0.1',resolve));
  const address=server.address(); const serverURL=`http://127.0.0.1:${address.port}/api/v1`;
  const client={auth:{apiKey:'management-secret'},serverURL};
  try {
    const success=await executeOperation(descriptor,{path:{hash:"a/b ?%#+ ü!'()*"},body:{limit:null}},client);
    assert.equal(success.status,200); assert.deepEqual(success.data,{ok:true});
    assert.deepEqual(requests[0],{url:'/api/v1/keys/a%2Fb%20%3F%25%23%2B%20%C3%BC%21%27%28%29%2A',method:'PATCH',authorization:'Bearer management-secret',accept:'application/json',contentType:'application/json',body:'{"limit":1.0000000000000000001}'});
    status=429; responseBody='{"error":"slow"}';
    let branded: unknown;
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client), (error:unknown)=>{branded=error;return isDeclaredApiError(error,operation)&&!isDeclaredApiError(error,source('/paths/~1credits/get'))});
    assert.throws(()=>{(branded as any).response.data.error='changed'},TypeError,'validated error data is immutable');
    (branded as any).operationSource=source('/paths/~1credits/get');
    assert.equal(isDeclaredApiError(branded,operation),true,'private operation identity survives public metadata mutation');
    assert.equal(isDeclaredApiError(branded,source('/paths/~1credits/get')),false,'metadata mutation cannot forge another operation');
    assert.equal(requests.length,2,'429 is not retried');
    for (const declaredStatus of [400,401,403,404,500]) {
      status=declaredStatus; responseBody='{"error":"declared"}';
      await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client),(error:unknown)=>isDeclaredApiError(error,operation)&&error.response.status===declaredStatus);
    }
    await assert.rejects(executeOperation(descriptor,{path:{hash:'.'},body:{limit:null}},client),(error:unknown)=>isSdkError(error,operation)&&error.kind==='request-representation');
    await assert.rejects(executeOperation(descriptor,{path:{hash:'\uD800'},body:{limit:null}},client),(error:unknown)=>isSdkError(error,operation)&&error.kind==='request-representation');
    await assert.rejects(executeOperation(descriptor,{path:{hash:''},body:{limit:null}},client),(error:unknown)=>isSdkError(error,operation)&&error.kind==='request-validation');
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},{...client,serverURL:`${serverURL}?unsafe=1`}),
      (error:unknown)=>isSdkError(error,operation)&&error.kind==='request-representation');
    await assert.rejects(executeOperation({...descriptor,maxResponseBytes:8},{path:{hash:'key'},body:{limit:null}},{...client,maxResponseBytes:9}),
      (error:unknown)=>isSdkError(error,operation)&&error.kind==='request-representation');
    const inherited=Object.create({apiKey:'prototype-secret'}) as {apiKey:string};
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},{...client,auth:inherited}),
      (error:unknown)=>isSdkError(error,operation)&&error.kind==='request-validation');
    for (const apiKey of ['', 'has space', 'line\nbreak', '=leading', 'middle=padding']) {
      await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},{...client,auth:{apiKey}}),
        (error:unknown)=>isSdkError(error,operation)&&error.kind==='request-validation'&&error.source?.pointer.endsWith('/securitySchemes/apiKey')===true);
    }
    status=201; responseBody='{"ok":true}';
    assert.equal((await executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client) as any).status,201);
    assert.equal(requests.length,8,'invalid requests never reach the server');
    const aborted=new AbortController(); aborted.abort(new Error('stop'));
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client,{signal:aborted.signal}),(error:unknown)=>isSdkError(error,operation)&&error.kind==='cancelled');
    assert.equal(requests.length,8);
    assert.equal(isDeclaredApiError({kind:'api-error',operationSource:operation,response:{}},operation),false,'shape forgery fails');
    status=429; responseBody='not json';
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client),(error:unknown)=>isSdkError(error,operation)&&error.kind==='response-decoding'&&error.source?.pointer===errorResponse.pointer);
    responseBody='{"wrong":true}';
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},client),(error:unknown)=>isSdkError(error,operation)&&error.kind==='response-decoding'&&!isDeclaredApiError(error,operation));
  } finally { await new Promise<void>(resolve=>server.close(()=>resolve())); }

  let cancelled=false;
  const stream=new ReadableStream<Uint8Array>({start(controller){controller.enqueue(new Uint8Array([123,34,120,34,58]));controller.enqueue(new Uint8Array(20));},cancel(){cancelled=true;}});
  const boundedFetch=async()=>new Response(stream,{status:200,headers:{'content-type':'application/json'}});
  await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},{auth:{apiKey:'x'},fetch:boundedFetch,maxResponseBytes:8}),
    (error:unknown)=>isSdkError(error,operation)&&error.kind==='resource-limit');
  assert.equal(cancelled,true,'reader is cancelled on byte budget failure');

  const headerAbort=new AbortController();
  const waitingFetch=(_input:unknown,init?:RequestInit)=>new Promise<Response>((_resolve,reject)=>init?.signal?.addEventListener('abort',()=>reject(init.signal?.reason),{once:true}));
  const waitingHeaders=executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},{auth:{apiKey:'x'},fetch:waitingFetch},{signal:headerAbort.signal});
  headerAbort.abort(new Error('headers stop'));
  await assert.rejects(waitingHeaders,(error:unknown)=>isSdkError(error,operation)&&error.kind==='cancelled');
  const returnedAborted=new AbortController();
  const nullAfterAbort=executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:async()=>{returnedAborted.abort(new Error('after headers'));return new Response(null,{status:200,headers:{'content-type':'application/json'}})}},
    {signal:returnedAborted.signal});
  await assert.rejects(nullAfterAbort,(error:unknown)=>isSdkError(error,operation)&&error.kind==='cancelled');

  let bodyCancelled=false;
  const pendingBody=new ReadableStream<Uint8Array>({pull(){return new Promise<void>(()=>{})},cancel(){bodyCancelled=true;}});
  const bodyAbort=new AbortController();
  const waitingBody=executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:async()=>new Response(pendingBody,{status:200,headers:{'content-type':'application/json'}})},{signal:bodyAbort.signal});
  await Promise.resolve(); bodyAbort.abort(new Error('body stop'));
  await assert.rejects(waitingBody,(error:unknown)=>isSdkError(error,operation)&&error.kind==='cancelled');
  assert.equal(bodyCancelled,true,'pending body reader is cancelled on abort'); assert.equal(pendingBody.locked,false,'reader lock is released');

  const failedStream=new ReadableStream<Uint8Array>({pull(controller){controller.error(new Error('wire broke'))}});
  await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:async()=>new Response(failedStream,{status:200,headers:{'content-type':'application/json'}})}),
    (error:unknown)=>isSdkError(error,operation)&&error.kind==='transport');
  assert.equal(failedStream.locked,false,'reader lock is released after stream rejection');

  const split=new TextEncoder().encode('{"ok":true,"text":"ü"}');
  const splitStream=new ReadableStream<Uint8Array>({start(controller){controller.enqueue(split.subarray(0,20));controller.enqueue(split.subarray(20,21));controller.enqueue(split.subarray(21));controller.close();}});
  const splitResult:any=await executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:async()=>new Response(splitStream,{status:200,headers:{'content-type':'application/json','content-length':'1'}})});
  assert.equal(splitResult.data.text,'ü','UTF-8 may cross chunks and Content-Length is only a hint');

  const unexpectedFetch=async()=>new Response('0123456789',{status:418,headers:{'content-type':'text/plain','content-length':'1'}});
  await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:unexpectedFetch,maxResponseBytes:20,maxErrorCaptureBytes:4}),
    (error:unknown)=>isSdkError(error,operation)&&error.kind==='unexpected-response'&&(error as any).rawCapture==='0123'&&(error as any).truncated===true);
  for (const malformed of ['application/json;garbage','application/json;charset=utf-8;charset=latin1']) {
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
      {auth:{apiKey:'x'},fetch:async()=>new Response('{"ok":true}',{status:200,headers:{'content-type':malformed}})}),
      (error:unknown)=>isSdkError(error,operation)&&error.kind==='unexpected-response'&&error.source?.pointer.endsWith('/responses/200')===true);
  }
  const quotedMedia=await executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{apiKey:'x'},fetch:async()=>new Response('{"ok":true}',{status:200,headers:{'content-type':'Application/JSON; charset="utf-8"'}})});
  assert.equal((quotedMedia as any).status,200);

  const namedDescriptor={...descriptor,security:{...descriptor.security,schemeName:'managementKey' as const}};
  let namedAuthorization='';
  await executeOperation(namedDescriptor,{path:{hash:'key'},body:{limit:null}},
    {auth:{managementKey:'named-secret'},fetch:async(_input,init)=>{namedAuthorization=new Headers(init?.headers).get('authorization')??'';return new Response('{"ok":true}',{status:200,headers:{'content-type':'application/json'}})}});
  assert.equal(namedAuthorization,'Bearer named-secret','credential lookup uses the exact source scheme name');

  let leaked:string|undefined;
  const destination=http.createServer((request:any,response:any)=>{leaked=request.headers.authorization;response.end('{"ok":true}')});
  await new Promise<void>(resolve=>destination.listen(0,'127.0.0.1',resolve));
  const destinationAddress=destination.address();
  const redirect=http.createServer((_request:any,response:any)=>{response.writeHead(302,{location:`http://127.0.0.1:${destinationAddress.port}/stolen`});response.end()});
  await new Promise<void>(resolve=>redirect.listen(0,'127.0.0.1',resolve));
  const redirectAddress=redirect.address();
  try {
    await assert.rejects(executeOperation(descriptor,{path:{hash:'key'},body:{limit:null}},
      {auth:{apiKey:'redirect-secret'},serverURL:`http://127.0.0.1:${redirectAddress.port}/api/v1`}),
      (error:unknown)=>isSdkError(error,operation)&&error.kind==='transport');
    assert.equal(leaked,undefined,'redirect policy never sends authorization to the second origin');
  } finally {
    await new Promise<void>(resolve=>redirect.close(()=>resolve()));
    await new Promise<void>(resolve=>destination.close(()=>resolve()));
  }
}
void main().catch((error:unknown)=>{setTimeout(()=>{throw error},0)});
"#;
