// Local GFM-style reading preview using the already installed documentation tools.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import MarkdownIt from '../../crates/suspect-codegen/tools/typescript-docs/node_modules/markdown-it/index.mjs';

const repo = fileURLToPath(new URL('../../', import.meta.url));
const root = path.resolve(process.argv[2] ?? '');
if (!root.startsWith(path.join(repo, 'target', 'sdk-demo-readme-20260911-'))) {
  throw new Error('Use the prepared demo root');
}
const directory = path.join(root, 'readme-check');
fs.mkdirSync(directory, { recursive: true });
const md = new MarkdownIt({ html: true, linkify: false });
const slugs = new Map();
md.renderer.rules.heading_open = (tokens, index, options, env, renderer) => {
  const title = tokens[index + 1].content;
  const slug = title.toLowerCase().replace(/[^\p{L}\p{N}_ -]/gu, '').replace(/ /g, '-');
  const count = slugs.get(slug) ?? 0;
  slugs.set(slug, count + 1);
  tokens[index].attrSet('id', count ? `${slug}-${count}` : slug);
  return renderer.renderToken(tokens, index, options);
};
md.renderer.rules.link_open = (tokens, index, options, env, renderer) => {
  const href = tokens[index].attrGet('href');
  if (href && !/^(?:[a-z]+:|#|\/)/i.test(href)) tokens[index].attrSet('href', `/${href}`);
  return renderer.renderToken(tokens, index, options);
};
const html = md.render(fs.readFileSync(path.join(repo, 'DEMO-README.md'), 'utf8'));
let number = 1;
while (fs.existsSync(path.join(directory, `render-${String(number).padStart(2, '0')}.html`))) number++;
const output = path.join(directory, `render-${String(number).padStart(2, '0')}.html`);
fs.writeFileSync(output, `<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>OpenAPI → 12 native SDKs</title>
<style>
:root{color-scheme:light dark}body{font:16px/1.65 system-ui,sans-serif;max-width:1080px;margin:48px auto;padding:0 28px;color:#18243a;background:#fff}
h1{font-size:40px;line-height:1.15}h2{margin-top:56px;border-top:1px solid #d8e1ec;padding-top:28px;font-size:28px}h3{margin-top:32px}
a{color:#1556b1;text-underline-offset:3px}code{font:13px/1.6 ui-monospace,monospace;overflow-wrap:anywhere;background:#eef3f9;padding:2px 4px;border-radius:4px}
pre{overflow:auto;padding:18px;background:#eef3f9;border:1px solid #d8e1ec;border-radius:9px}pre code{padding:0;white-space:pre;overflow-wrap:normal}
table{display:block;overflow:auto;border-collapse:collapse;width:100%;font-size:14px}th,td{border:1px solid #d8e1ec;padding:10px 12px;vertical-align:top;text-align:left}th{background:#edf3fa}
@media(prefers-color-scheme:dark){body{color:#dce6f5;background:#101824}a{color:#81b7ff}code,pre,th{background:#1b2b40}h2,th,td,pre{border-color:#364960}}
</style><main>${html}</main></html>`);
console.log(output);
