// Actual browser execution of source-compiled v3 programs and native SDK calls.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const dist = resolve('dist'), requests = [], paths = [];
const html = String.raw`<!doctype html><html><head><meta charset="utf-8"><title>Resource SDK browser</title></head>
<body data-sdk-status="pending"><pre id="result"></pre><script type="module">
import { createClient, operations, codecs, JsonNumber, ModelCodecError, parseJson } from '/sdk/source/index.js';
const check=(condition,label)=>{if(!condition)throw new Error(label);};
const reject=async(call,kind)=>{try{await call();}catch(error){check(operations.isSdkError(error)&&error.kind===kind,kind+': '+error);return;}throw new Error('expected '+kind);};
try{
    const cases=await(await fetch('/vectors.json')).json();
    for(const test of cases){
        const program=await import('/sdk/vector'+test.program+'.js');
        const root=program.validationRoots[test.root],value=parseJson(test.instanceJson);
        const result=program.validate(root,value);
        check(result.kind===test.expected,test.id+': '+JSON.stringify(result));
        check(JSON.stringify(program['validateRoot'+test.root](root,value))===JSON.stringify(result),test.id+': root-specific graph');
    }
    const client=createClient({serverURL:location.origin});
    const strict=(await client.strict({body:{value:1n,children:[{value:9007199254740993n}]}})).data;
    check(strict.value===9007199254740993n&&strict.children[0].value.toString()==='2','dynamic exact model representations');
    check((await client.loose({body:{value:1n,children:[{value:2n,extra:true}]}})).data.children[0].extra===true,'unentered strict binding stays inert');
    check((await client.exact({body:{amount:JsonNumber.parse('9007199254740993.25')}})).data.amount.toString()==='9007199254740993.25','exact resource-bound decimal');
    check((await client.contextual({body:{value:7n}})).data.value.toString()==='7','failed condition cannot overwrite another dynamic-context projection trace');
    await client.strict({body:{value:1n}}, {server:{documentURL:location.origin+'/spec/entry.json'}});
    await reject(()=>client.strict({body:{value:1n}}, {server:{documentURL:location.origin+'/spec/entry.json',variables:{base:'%2E%2E/kept'}}}),'request-representation');
    await reject(()=>client.strict({body:{value:1n,children:[{value:2n,extra:true}]}}),'request-validation');
    await reject(()=>client.strict({body:{value:1n,children:[null]}}),'request-validation');
    strict.children[0].extra=true;let mutated=false;
    try{codecs.StrictCodec.encode(strict);}catch(error){mutated=error instanceof ModelCodecError&&error.kind==='invalid'&&error.findings.some(finding=>finding.source.document==='https://physical.test/api.json'&&finding.source.pointer==='/components/schemas/Strict/unevaluatedProperties'&&finding.instancePath==='/children/0/extra');}
    check(mutated,'encode revalidates mutation with physical finding source');
    check(!Object.hasOwn(codecs.StrictCodec.decode('{"value":1}'),'children'),'absence survives decode');
    const abort=new AbortController();abort.abort();await reject(()=>client.strict({body:{value:1n}},{signal:abort.signal}),'cancelled');
    await reject(()=>client.strict({body:{value:0n}}),'response-decoding');
    const resource=operations.operationMetadata.strict.body.media[0].source.terminal_resource;
    check(resource.canonical_uri==='https://logical.test/catalog/api.json#revision'&&resource.source.source.document==='https://physical.test/api.json'&&Object.isFrozen(resource),'logical and physical metadata stay distinct');
    const evidence=await(await fetch('/evidence')).json();check(evidence.calls===6&&evidence.exact&&evidence.relative,'native HTTP controls, physical-base override and exact request bytes');
    document.body.dataset.sdkStatus='passed';document.getElementById('result').textContent=JSON.stringify({agent:navigator.userAgent,vectors:cases.length,...evidence});
}catch(error){document.body.dataset.sdkStatus='failed';document.getElementById('result').textContent=String(error.stack||error);}
</script></body></html>`;
const server = createServer(async (request, response) => {
    try {
        const url = new URL(request.url, 'http://fixture');
        if (url.pathname === '/') { response.writeHead(200, { 'content-type': 'text/html' }); response.end(html); return; }
        if (url.pathname.startsWith('/sdk/') && !url.pathname.includes('..')) { response.writeHead(200, { 'content-type': 'text/javascript' }); response.end(await readFile(join(dist, url.pathname.slice(5)))); return; }
        if (url.pathname === '/vectors.json') { response.writeHead(200, { 'content-type': 'application/json' }); response.end(await readFile('vectors.json')); return; }
        if (url.pathname === '/evidence') { response.writeHead(200, { 'content-type': 'application/json' }); response.end(JSON.stringify({ calls: requests.length, exact: requests[0] === '{"value":1,"children":[{"value":9007199254740993}]}', relative: paths[4] === '/api/strict' })); return; }
        if (['/strict', '/loose', '/exact', '/contextual', '/api/strict'].includes(url.pathname)) {
            const chunks = []; for await (const chunk of request) chunks.push(chunk);
            const body = Buffer.concat(chunks).toString(); requests.push(body); paths.push(request.url);
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end(url.pathname === '/exact' ? '{"amount":9007199254740993.25}' : url.pathname === '/contextual' ? '{"value":7}' : ['/strict', '/api/strict'].includes(url.pathname) && JSON.parse(body).value !== 0 ? '{"value":9007199254740993,"children":[{"value":2}]}' : '{"value":1,"children":[{"value":2,"extra":true}]}');
            return;
        }
        response.writeHead(404); response.end();
    } catch (error) { response.writeHead(500); response.end(String(error.stack)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const profile = await mkdtemp(join(process.cwd(), 'browser-profile-'));
const pageURL = `http://127.0.0.1:${server.address().port}/`;
const chrome = process.env.SUSPECT_CHROMIUM ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const child = spawn(chrome, ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-extensions', '--password-store=basic', '--use-mock-keychain', '--remote-debugging-port=0', `--user-data-dir=${profile}`, pageURL]);
const stopped = new Promise(resolve => child.on('close', resolve));
let stderr = '';
const debugging = new Promise((resolve, reject) => {
    child.stderr.on('data', chunk => { stderr += chunk; const match = /DevTools listening on (ws:\/\/[^\s]+)/.exec(stderr); if (match) resolve(new URL(match[1])); });
    child.on('error', reject); child.on('close', () => reject(new Error(stderr)));
});
const timeout = setTimeout(() => child.kill('SIGKILL'), 30000);
let socket;
try {
    const address = await debugging; let target;
    for (let attempt = 0; attempt < 100; attempt++) {
        const targets = await (await fetch(`http://${address.host}/json/list`)).json();
        target = targets.find(target => target.type === 'page' && target.url === pageURL);
        if (target) break;
        await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert.ok(target, 'Chromium page target'); socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    const waiting = new Map(); let requestID = 0;
    socket.addEventListener('message', event => { const message = JSON.parse(event.data), request = waiting.get(message.id); if (!request) return; waiting.delete(message.id); if (message.error) request.reject(message.error); else request.resolve(message.result); });
    socket.addEventListener('close', () => { for (const request of waiting.values()) request.reject(new Error(stderr)); waiting.clear(); }, { once: true });
    let evaluated;
    for (let attempt = 0; attempt < 10; attempt++) {
        const id = ++requestID, result = new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
        socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { awaitPromise: true, returnByValue: true,
            expression: `new Promise(resolve=>{const check=()=>{const status=document.body?.dataset.sdkStatus;if(status==='passed'||status==='failed'){observer.disconnect();resolve({status,content:document.getElementById('result').textContent});}};const observer=new MutationObserver(check);observer.observe(document,{subtree:true,childList:true,attributes:true});check();})`,
        } }));
        try { evaluated = await result; break; } catch (error) { if (error.code !== -32000 || !error.message.includes('context')) throw error; await new Promise(resolve => setTimeout(resolve, 20)); }
    }
    assert.equal(evaluated?.result?.value?.status, 'passed', JSON.stringify(evaluated) + '\n' + stderr);
    assert.equal(JSON.parse(evaluated.result.value.content).vectors, 44);
    console.log('browser-v3-resources-passed', evaluated.result.value.content);
} finally {
    clearTimeout(timeout); socket?.close(); child.kill(); await stopped;
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
}
