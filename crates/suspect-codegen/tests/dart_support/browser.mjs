// Run exact previously compiled Dart consumers in a real headless Chrome page.
// Usage: node browser.mjs <evidence-root> <manifest.json>; Node >=22, no packages.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { createWriteStream, existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve, join, sep } from 'node:path';

const root = resolve(process.argv[2]);
const cases = JSON.parse(readFileSync(process.argv[3], 'utf8'));
const directory = mkdtempSync(join(root, 'browser-'));
const chrome = process.env.SUSPECT_DART_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const scripts = cases.map(test => {
  const path = resolve(root, test.script);
  if (!path.startsWith(root + sep)) throw Error('script is outside the evidence root');
  return readFileSync(path);
});
const server = createServer((request, response) => {
  const match = /^\/(\d+)\.(html|js)$/.exec(request.url);
  if (!match || !cases[Number(match[1])]) { response.writeHead(404).end(); return; }
  const index = Number(match[1]);
  if (match[2] === 'js') { response.writeHead(200, { 'Content-Type': 'text/javascript' }).end(scripts[index]); return; }
  const marker = JSON.stringify(cases[index].marker).replaceAll('<', '\\u003c');
  response.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' }).end(`<!doctype html>
<meta charset="utf-8"><title>Dart v2 native browser witness</title><h1>Dart v2 native browser witness</h1><pre id="result">Running…</pre>
<script>
window.dartGate={passed:false,logs:[],errors:[]};
function show(){ document.querySelector('#result').textContent=JSON.stringify(dartGate,null,2); }
const original=console.log.bind(console);
console.log=(...args)=>{ original(...args); const text=args.map(String).join(' '); dartGate.logs.push(text); if(text.includes(${marker}))dartGate.passed=true; show(); };
addEventListener('error',e=>{dartGate.errors.push(e.message);show();});
addEventListener('unhandledrejection',e=>{dartGate.errors.push(String(e.reason));show();});
</script><script src="/${index}.js"></script>`);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const url = `http://127.0.0.1:${server.address().port}`;
const child = spawn(chrome, ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-extensions', '--remote-debugging-port=0', '--user-data-dir=' + join(directory, 'profile'), 'about:blank'], { detached: true, stdio: ['ignore', 'ignore', 'pipe'] });
child.stderr.pipe(createWriteStream(join(directory, 'chrome-stderr.log')));
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const report = { passed: false, cases: [], errors: [] };
try {
  const active = join(directory, 'profile', 'DevToolsActivePort');
  for (let i=0; i<100 && !existsSync(active); i++) await delay(100);
  const port = readFileSync(active, 'utf8').split('\n')[0];
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  report.browser = version.Browser;
  for (const [index, test] of cases.entries()) {
    const page = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, { method: 'PUT' })).json();
    const socket = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve,reject) => { socket.addEventListener('open',resolve,{once:true}); socket.addEventListener('error',reject,{once:true}); });
    let next = 0; const pending = new Map();
    socket.addEventListener('message', event => {
      const value = JSON.parse(event.data);
      const item = pending.get(value.id);
      if (item) { pending.delete(value.id); value.error ? item.reject(value.error) : item.resolve(value.result); }
    });
    const call = (method,params={}) => new Promise((resolve,reject) => {
      const id=++next; pending.set(id,{resolve,reject}); socket.send(JSON.stringify({id,method,params}));
    });
    const item = { ...test, sha256: createHash('sha256').update(scripts[index]).digest('hex'), passed:false, logs:[], errors:[] };
    try {
      await call('Page.enable'); await call('Runtime.enable'); await call('Page.navigate',{url:`${url}/${index}.html`});
      for(let i=0; i<150; i++) {
        await delay(100);
        const state = (await call('Runtime.evaluate',{expression:'window.dartGate || null',returnByValue:true})).result.value;
        if(state) { Object.assign(item,state); if(state.passed || state.errors.length) break; }
      }
      const dom = (await call('Runtime.evaluate',{expression:'document.documentElement.outerHTML',returnByValue:true})).result.value;
      writeFileSync(join(directory, `${index}-dom.html`), dom);
      if(!item.passed) item.errors.push('completion marker was not observed');
    } finally { socket.close(); }
    report.cases.push(item);
  }
  report.passed = report.cases.length === cases.length && report.cases.every(test=>test.passed && test.errors.length===0);
} catch(error) { report.errors.push(String(error)); }
finally {
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  server.closeAllConnections(); await new Promise(resolve=>server.close(resolve));
  writeFileSync(join(directory, 'report.json'), JSON.stringify(report,null,2)+'\n');
}
console.log(JSON.stringify({ directory, ...report },null,2));
process.exit(report.passed && report.errors.length===0 ? 0 : 1);
