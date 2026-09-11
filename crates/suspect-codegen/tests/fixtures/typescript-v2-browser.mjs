// Native browser witness over the generated SDK and source-compiled programs.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdtemp } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const dist = resolve(process.argv[2] ?? 'dist');
const requests = [];
const replies = {
    '/conditional': '{"kind":"number","number":9007199254740993}',
    '/dependency': '{"flag":null,"peer":"present"}',
    '/patterns': '{"name":"reply","x-count":9007199254740993,"__proto__":"kept"}',
    '/sequence': '["prefix",1e3,"tail"]',
};
const html = String.raw`<!doctype html><html><head><meta charset="utf-8"><title>Scoped SDK browser</title></head>
<body data-sdk-status="pending"><pre id="result"></pre><script type="module">
import { createClient, operations, codecs, JsonNumber, ModelCodecError, parseJson } from '/sdk/source/index.js';
const check = (condition, label) => { if (!condition) throw new Error(label); };
const rejects = async (call, kind) => {
    try { await call(); } catch (error) {
        check(operations.isSdkError(error) && error.kind === kind, kind + ': ' + error);
        return;
    }
    throw new Error('expected ' + kind);
};
try {
    const cases = await (await fetch('/vectors.json')).json();
    for (const test of cases) {
        const program = await import('/sdk/vector' + test.program + '.js');
        const root = program.validationRoots[0], value = parseJson(test.instanceJson);
        const result = program.validate(root, value);
        check(result.kind === test.expected, test.id + ': ' + JSON.stringify(result));
        check(JSON.stringify(program.validateRoot0(root, value)) === JSON.stringify(result), test.id + ': root slice');
        if (test.source) {
            const findings = result.kind === 'invalid' ? result.findings : [result.finding];
            check(findings.some(finding => finding.source.document === root.document && finding.source.pointer === test.source && finding.instancePath === test.instancePath), test.id + ': source/path');
        }
    }
    const client = createClient({ serverURL: location.origin });
    check((await client.conditional({ body: { kind: 'text', text: 'native' } })).data.number === 9007199254740993n, 'native integer');
    check((await client.dependency({ body: { flag: null, peer: 'present' } })).data.flag === null, 'null is present');
    const patterns = (await client.patterns({ body: { name: 'native', 'x-count': 5n, ['__proto__']: 'owned' } })).data;
    check(patterns['x-count'].toString() === '9007199254740993' && patterns.__proto__ === 'kept' && Object.getPrototypeOf(patterns) === null, 'faithful pattern extras');
    const sequence = (await client.sequence({ body: ['prefix', JsonNumber.parse('9007199254740993'), 'tail'] })).data;
    check(sequence[1].toString() === '1e3', 'prefix/contains exact JSON carrier');
    for (const call of [
        () => client.conditional({ body: { kind: 'text' } }),
        () => client.dependency({ body: { flag: null } }),
        () => client.dependency({ body: { other: true } }),
        () => client.patterns({ body: { name: 'x', 'x-amount': 1n } }),
        () => client.patterns({ body: { name: 'x', 'x-denied': 1n } }),
        () => client.patterns({ body: { name: 'x', extra: true } }),
        () => client.sequence({ body: ['prefix', false] }),
        () => client.sequence({ body: ['prefix', 1n, 2n, 3n] }),
    ]) await rejects(call, 'request-validation');
    patterns['x-count'] = JsonNumber.parse('-1');
    let mutationRejected = false;
    try { codecs.PatternsCodec.encode(patterns); } catch (error) { mutationRejected = error instanceof ModelCodecError && error.kind === 'invalid'; }
    check(mutationRejected, 'encode revalidates mutation');
    check(!Object.hasOwn(codecs.DependencyCodec.decode('{}'), 'flag'), 'absence is retained');
    const abort = new AbortController(); abort.abort();
    await rejects(() => client.dependency({ body: {} }, { signal: abort.signal }), 'cancelled');
    await rejects(() => client.patterns({ body: { name: 'invalid-response' } }), 'response-decoding');
    const evidence = await (await fetch('/evidence')).json();
    check(evidence.calls === 5 && evidence.exactBody, 'wire evidence and pre-transport controls');
    document.body.dataset.sdkStatus = 'passed';
    document.getElementById('result').textContent = JSON.stringify({ agent: navigator.userAgent, vectors: cases.length, ...evidence });
} catch (error) {
    document.body.dataset.sdkStatus = 'failed';
    document.getElementById('result').textContent = String(error.stack || error);
}
</script></body></html>`;

const server = createServer(async (request, response) => {
    try {
        const url = new URL(request.url, 'http://fixture');
        if (url.pathname === '/') { response.writeHead(200, { 'content-type': 'text/html' }); response.end(html); return; }
        if (url.pathname.startsWith('/sdk/') && !url.pathname.includes('..')) {
            response.writeHead(200, { 'content-type': 'text/javascript' });
            response.end(await readFile(join(dist, url.pathname.slice(5))));
            return;
        }
        if (url.pathname === '/vectors.json') {
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end(await readFile('vectors.json'));
            return;
        }
        if (url.pathname === '/evidence') {
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end(JSON.stringify({ calls: requests.length, exactBody: requests[3]?.body === '["prefix",9007199254740993,"tail"]' }));
            return;
        }
        if (Object.hasOwn(replies, url.pathname)) {
            const chunks = [];
            for await (const chunk of request) chunks.push(chunk);
            const body = Buffer.concat(chunks).toString();
            requests.push({ path: url.pathname, body });
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end(url.pathname === '/patterns' && JSON.parse(body).name === 'invalid-response' ? '{"name":"reply","extra":true}' : replies[url.pathname]);
            return;
        }
        response.writeHead(404); response.end();
    } catch (error) { response.writeHead(500); response.end(String(error.stack)); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const profile = await mkdtemp(join(process.cwd(), 'browser-profile-'));
const chrome = process.env.SUSPECT_CHROMIUM ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const pageURL = `http://127.0.0.1:${server.address().port}/`;
const child = spawn(chrome, ['--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-extensions', '--password-store=basic', '--use-mock-keychain', '--remote-debugging-port=0', `--user-data-dir=${profile}`, pageURL]);
const stopped = new Promise(resolve => child.on('close', resolve));
let stderr = '';
const debugging = new Promise((resolve, reject) => {
    child.stderr.on('data', chunk => { stderr += chunk; const match = /DevTools listening on (ws:\/\/[^\s]+)/.exec(stderr); if (match) resolve(new URL(match[1])); });
    child.on('error', reject);
    child.on('close', () => reject(new Error(stderr)));
});
const timeout = setTimeout(() => child.kill('SIGKILL'), 30000);
let socket;
try {
    const address = await debugging;
    let target;
    for (let attempt = 0; attempt < 100; attempt++) {
        const targets = await (await fetch(`http://${address.host}/json/list`)).json();
        target = targets.find(target => target.type === 'page' && target.url === pageURL);
        if (target) break;
        await new Promise(resolve => setTimeout(resolve, 20));
    }
    assert.ok(target, 'Chromium page target');
    socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, { once: true }); socket.addEventListener('error', reject, { once: true }); });
    const waiting = new Map();
    let requestID = 0;
    socket.addEventListener('message', event => {
        const message = JSON.parse(event.data), request = waiting.get(message.id);
        if (!request) return;
        waiting.delete(message.id);
        if (message.error) request.reject(message.error); else request.resolve(message.result);
    });
    socket.addEventListener('close', () => { for (const request of waiting.values()) request.reject(new Error(stderr)); waiting.clear(); }, { once: true });
    let evaluated;
    for (let attempt = 0; attempt < 10; attempt++) {
        const id = ++requestID, result = new Promise((resolve, reject) => waiting.set(id, { resolve, reject }));
        socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: {
            awaitPromise: true, returnByValue: true,
            expression: `new Promise(resolve=>{const check=()=>{const status=document.body?.dataset.sdkStatus;if(status==='passed'||status==='failed'){observer.disconnect();resolve({status,content:document.getElementById('result').textContent});}};const observer=new MutationObserver(check);observer.observe(document,{subtree:true,childList:true,attributes:true});check();})`,
        } }));
        try { evaluated = await result; break; } catch (error) {
            if (error.code !== -32000 || !error.message.includes('context')) throw error;
            await new Promise(resolve => setTimeout(resolve, 20));
        }
    }
    assert.equal(evaluated?.result?.value?.status, 'passed', JSON.stringify(evaluated) + '\n' + stderr);
    assert.equal(requests.length, 5);
    assert.equal(JSON.parse(evaluated.result.value.content).vectors, 32);
    console.log('browser-v2-scoped-passed', evaluated.result.value.content);
} finally {
    clearTimeout(timeout); socket?.close(); child.kill(); await stopped;
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
}
