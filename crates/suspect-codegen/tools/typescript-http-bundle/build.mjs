import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import * as esbuild from 'esbuild';

assert.equal(process.versions.node, '22.23.1', 'bundle evidence requires pinned Node 22.23.1');
const [work, packageName, codecName, reportPath] = process.argv.slice(2);
assert.ok(work && packageName && codecName && reportPath, 'usage: build.mjs WORK PACKAGE CODEC REPORT');
await mkdir(work, { recursive: true });

const entries = {
  json: `import {parseJson} from ${JSON.stringify(`${packageName}/json`)};\nexport function run(n){let v;for(let i=0;i<n;i++)v=parseJson('{"n":1.0000000000000000001}');return v}`,
  codec: `import {${codecName}} from ${JSON.stringify(`${packageName}/codecs`)};\nexport function run(n){let v;for(let i=0;i<n;i++)v=${codecName}.decode('{"error":{"code":400,"message":"Invalid request parameters"}}');return v}`,
  operation: `import {getCredits} from ${JSON.stringify(`${packageName}/operations`)};\nexport async function run(n){const client={auth:{apiKey:'benchmark-token'},fetch:async()=>new Response('{"data":{"total_credits":1.0000000000000000001,"total_usage":0}}',{status:200,headers:{'content-type':'application/json'}})};let v;for(let i=0;i<n;i++)v=await getCredits(client,{});return v}`,
  allOperations: `import {getCredits,createKeys,updateKeys} from ${JSON.stringify(`${packageName}/operations`)};\nexport const selected=[getCredits,createKeys,updateKeys]`,
};
const evidence = { node: process.versions.node, esbuild: esbuild.version, selectedCodec: codecName, bundles: {}, benchmark: {} };
const stableInput = input => {
  const normalized = input.replaceAll('\\', '/');
  const installed = normalized.indexOf('/node_modules/');
  return installed < 0 ? `entry/${path.basename(normalized)}` : normalized.slice(installed + 1);
};
for (const [name, source] of Object.entries(entries)) {
  const entry = path.join(work, `${name}.mjs`);
  const outfile = path.join(work, `${name}.bundle.mjs`);
  await writeFile(entry, source);
  const result = await esbuild.build({ entryPoints:[entry], outfile, bundle:true, format:'esm', platform:'node', target:'node22', treeShaking:true, minify:true, metafile:true });
  const bytes = await readFile(outfile);
  const output = Object.values(result.metafile.outputs)[0];
  evidence.bundles[name] = {
    bytes: bytes.byteLength,
    gzipBytes: gzipSync(bytes, { level: 9 }).byteLength,
    inputs: Object.keys(result.metafile.inputs).map(stableInput).sort(),
    bytesByInput: Object.fromEntries(Object.entries(output.inputs).map(([input, detail]) => [stableInput(input), detail.bytesInOutput]).sort(([left],[right]) => left.localeCompare(right))),
    retainsCreateKeys: bytes.includes(Buffer.from('createKeys')) || bytes.includes(Buffer.from('/keys')),
    retainsUpdateKeys: bytes.includes(Buffer.from('updateKeys')) || bytes.includes(Buffer.from('/keys/{hash}')),
  };
}
for (const [name, iterations] of [['json', 2000], ['codec', 1000], ['operation', 200]]) {
  const module = await import(`${pathToFileURL(path.join(work, `${name}.bundle.mjs`)).href}?run=${Date.now()}`);
  await module.run(1);
  const start = process.hrtime.bigint();
  await module.run(iterations);
  const elapsed = process.hrtime.bigint() - start;
  evidence.benchmark[name] = { iterations, elapsedNs: Number(elapsed), nsPerOperation: Number(elapsed) / iterations };
}
let start = process.hrtime.bigint();
const installedCodecs = path.join(path.dirname(work), 'node_modules', ...packageName.split('/'), 'dist/model-codecs.js');
const unbundledCodecs = await import(pathToFileURL(installedCodecs).href);
evidence.benchmark.unbundledCodecImport = { elapsedNs: Number(process.hrtime.bigint() - start) };
start = process.hrtime.bigint();
unbundledCodecs[codecName].decode('{"error":{"code":400,"message":"Invalid request parameters"}}');
evidence.benchmark.unbundledCodecFirstCall = { elapsedNs: Number(process.hrtime.bigint() - start) };
await writeFile(reportPath, `${JSON.stringify(evidence, null, 2)}\n`);
