// Portable ESM proof: all SDK calls are captured, never sent to account endpoints.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { spawn } from 'node:child_process';

const html = String.raw`<!doctype html><html><head><meta charset="utf-8"><title>Credential environment portability</title></head>
<body data-sdk-status="pending"><pre id="result"></pre><script type="module">
const check=(value,message)=>{if(!value)throw new Error(message);};
let reads=0;
Object.defineProperty(globalThis,'process',{configurable:true,get(){reads++;throw new Error('host-value-must-not-leak');}});
try {
    const synthetic=await import('/synthetic/source/index.js');
    const openrouter=await import('/openrouter/source/index.js');
    check(reads===0,'ESM import read an environment value');
    delete globalThis.process;
    const responses=await(await fetch('/responses.json')).json();
    const requests=[];
    globalThis.fetch=async(input,init)=>{
        const url=String(input),headers=new Headers(init.headers);requests.push({url,headers});
        check(url.startsWith('https://api.example.test/v1/')||url==='https://openrouter.ai/api/v1/key','source HTTPS endpoint');
        return new Response(url==='https://openrouter.ai/api/v1/key'?responses.getCurrentKey:'{"ok":true}',{status:200,headers:{'content-type':'application/json'}});
    };
    const reject=async(action,isSdkError)=>{const before=requests.length;let rejected=false;try{await action();}catch(error){rejected=isSdkError(error)&&error.kind==='request-validation'&&!String(error.stack).includes('host-value-must-not-leak');}check(rejected,'missing credential must fail clearly');check(requests.length===before,'missing credential reached HTTP');};
    const absent=synthetic.createClient();
    await absent.anonymous();await absent.optional();
    check(requests.every(request=>request.headers.get('Authorization')===null),'anonymous call attached credentials');
    await reject(()=>absent.bearerOnly(),synthetic.operations.isSdkError);
    await reject(()=>absent.either(),synthetic.operations.isSdkError);
    await reject(()=>absent.both(),synthetic.operations.isSdkError);
    await reject(()=>absent.optional({}, {securityAlternative:1}),synthetic.operations.isSdkError);
    Object.defineProperty(globalThis,'process',{configurable:true,get(){reads++;throw new Error('host-value-must-not-leak');}});
    const explicit=synthetic.createClient({auth:{bearer:'browser-explicit'}});
    await explicit.bearerOnly();check(requests.at(-1).headers.get('Authorization')==='Bearer browser-explicit','explicit credentials');
    await reject(()=>explicit.both(),synthetic.operations.isSdkError);
    for(const auth of [undefined,{}, {bearer:undefined}, {bearer:''}])await reject(()=>synthetic.createClient({auth}).bearerOnly(),synthetic.operations.isSdkError);
    let nullRejected=false;try{synthetic.createClient({auth:null});}catch(error){nullRejected=error instanceof TypeError;}check(nullRejected,'null explicit constructor');
    check(reads===0,'explicit auth triggered host lookup');
    const unavailable=synthetic.createClient({});check(reads===1,'one bounded unavailable-host lookup at creation');
    delete globalThis.process;
    await unavailable.anonymous();await reject(()=>unavailable.bearerOnly(),synthetic.operations.isSdkError);
    await reject(()=>openrouter.createClient().getCurrentKey(),openrouter.operations.isSdkError);
    await reject(()=>openrouter.createClient({auth:undefined}).getCurrentKey(),openrouter.operations.isSdkError);
    const current=await openrouter.createClient({auth:{apiKey:'browser-current-key'}}).getCurrentKey();
    check(current.status===200&&current.data.data.is_management_key===false&&current.data.data.limit_remaining.toString()==='74.5','actual current-key schema decode');
    check(requests.at(-1).url==='https://openrouter.ai/api/v1/key'&&requests.at(-1).headers.get('Authorization')==='Bearer browser-current-key','source bearer binding and default HTTPS');
    const report={agent:navigator.userAgent,requests:requests.length,importReads:0,unavailableHost:true,explicitPrecedence:true,anonymous:true,currentKey:true,controlledTransport:true,liveAccount:false};
    document.body.dataset.sdkStatus='passed';document.getElementById('result').textContent=JSON.stringify(report);
}catch(error){document.body.dataset.sdkStatus='failed';document.getElementById('result').textContent=String(error.stack||error);}
</script></body></html>`;
const server = createServer(async (request, response) => {
    try {
        const url = new URL(request.url, 'http://fixture');
        if (url.pathname === '/') { response.writeHead(200, { 'content-type': 'text/html' }); response.end(html); return; }
        if (url.pathname === '/responses.json') { response.writeHead(200, { 'content-type': 'application/json' }); response.end(await readFile('responses.json')); return; }
        const match = /^\/(synthetic|openrouter)\/(.+)$/.exec(url.pathname);
        if (match && !match[2].includes('..')) { response.writeHead(200, { 'content-type': 'text/javascript' }); response.end(await readFile(join(match[1], 'typescript/dist', match[2]))); return; }
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
    await writeFile('browser-result.json', evaluated.result.value.content + '\n');
    console.log('credential-env-browser-passed', evaluated.result.value.content);
} finally {
    clearTimeout(timeout); socket?.close(); child.kill(); await stopped;
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
}
