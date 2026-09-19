// Independent YAML 1.2 AST oracle. Numeric scalar source text bypasses JS floats.
import assert from 'node:assert/strict';
import {readFile,writeFile,realpath} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
import {parseDocument,isScalar,isMap,isSeq,isAlias} from 'yaml';

const own=JSON.parse(await readFile(new URL('./package.json',import.meta.url),'utf8'));
assert.equal(process.versions.node,own.engines.node);
const yaml=JSON.parse(await readFile(new URL('./node_modules/yaml/package.json',import.meta.url),'utf8'));
assert.equal(yaml.version,own.devDependencies.yaml);
assert.equal(typeof JSON.rawJSON,'function','Node must preserve exact JSON numeric tokens');
assert.equal(process.argv.length,4,'usage: node normalize.mjs INPUT_YAML NEW_OUTPUT_JSON');
const input=await realpath(process.argv[2]);
const bytes=await readFile(input);
const document=parseDocument(bytes.toString('utf8'),{version:'1.2',schema:'core',uniqueKeys:true,intAsBigInt:true});
assert.equal(document.errors.length,0,document.errors.map(error=>error.message).join('\n'));
assert.equal(document.warnings.length,0,document.warnings.map(error=>error.message).join('\n'));

function number(source) {
  let text=source.replaceAll('_','');
  let sign='';
  if(text[0]==='+' || text[0]==='-') {sign=text[0]==='-'?'-':'';text=text.slice(1);}
  if(/^0[xob]/iu.test(text)) return JSON.rawJSON(sign+BigInt(text).toString());
  if(text.startsWith('.')) text='0'+text;
  text=text.replace(/^0+(?=\d)/u,'').replace(/\.(?=[eE]|$)/u,'.0');
  const token=sign+text;
  assert.match(token,/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/u,'non-JSON numeric value');
  return JSON.rawJSON(token);
}
let visits=0;
function value(node,depth=0) {
  assert.ok(++visits<=1_000_000 && depth<=256,'oracle resource ceiling');
  if(node===null) return null;
  assert.ok(!isAlias(node),'this oracle requires alias-free source snapshots');
  if(isScalar(node)) {
    if(typeof node.value==='number' || typeof node.value==='bigint') return number(node.source);
    assert.ok(node.value===null || typeof node.value==='string' || typeof node.value==='boolean','unsupported scalar');
    return node.value;
  }
  if(isSeq(node)) return node.items.map(item=>value(item,depth+1));
  assert.ok(isMap(node),'unsupported YAML node');
  const result=Object.create(null);
  for(const pair of node.items) {
    assert.ok(isScalar(pair.key) && typeof pair.key.value==='string','OpenAPI mapping keys must be strings');
    assert.ok(!Object.hasOwn(result,pair.key.value),'duplicate decoded mapping key');
    result[pair.key.value]=value(pair.value,depth+1);
  }
  return result;
}
const output=JSON.stringify(value(document.contents),null,2)+'\n';
await writeFile(process.argv[3],output,{flag:'wx'});
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
await writeFile(process.argv[3]+'.meta.json',JSON.stringify({format:'suspect.yaml-oracle.v1',input,inputSha256:sha(bytes),outputSha256:sha(output),node:process.version,yaml:yaml.version,visits,scriptSha256:sha(await readFile(fileURLToPath(import.meta.url))),lockSha256:sha(await readFile(new URL('./package-lock.json',import.meta.url)))},null,2)+'\n',{flag:'wx'});
console.log('independent-yaml-oracle',visits);
