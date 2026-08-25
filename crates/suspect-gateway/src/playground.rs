//! Interactive contract playground: a browser UI served by the gateway.
//!
//! Available at `/playground` when the gateway runs. Presents every
//! operation from the loaded spec as a searchable palette; selecting one
//! generates a ready-to-send request template from schema defaults. The
//! page dispatches requests through the gateway itself, so responses are
//! validated and journaled exactly like live traffic.

use suspect_ir::IrSpec;

/// The single-file playground HTML app.
#[must_use]
pub fn playground_html(spec: &IrSpec) -> String {
    let ops: Vec<String> =
        spec.operations
            .iter()
            .map(|op| {
                let id = op.id.clone().unwrap_or_else(|| {
                    format!("{} {}", op.method.as_str().to_uppercase(), op.path)
                });
                let summary = op.summary.clone().unwrap_or_default();
                let params: Vec<String> = op
                    .parameters
                    .iter()
                    .map(|p| {
                        format!(
                            r#"{{"name":{},"in":{},"required":{}}}"#,
                            json_str(&p.name),
                            json_str(match p.location {
                                suspect_ir::ParamIn::Query => "query",
                                suspect_ir::ParamIn::Header => "header",
                                suspect_ir::ParamIn::Path => "path",
                                suspect_ir::ParamIn::Cookie => "cookie",
                            }),
                            p.required
                        )
                    })
                    .collect();
                format!(
                    r#"{{"id":{},"method":{},"path":{},"summary":{},"params":[{}]}}"#,
                    json_str(&id),
                    json_str(op.method.as_str()),
                    json_str(&op.path),
                    json_str(&summary),
                    params.join(",")
                )
            })
            .collect();

    let title = json_str(&spec.title);
    let ops_json = ops.join(",");

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title} — Contract Playground</title>
<style>
  :root {{ color-scheme: dark; }}
  * {{ box-sizing: border-box; }}
  body {{ margin: 0; font: 14px/1.5 ui-monospace, monospace; background: #0d1117; color: #c9d1d9; }}
  header {{ padding: 12px 20px; border-bottom: 1px solid #21262d; display: flex; gap: 16px; align-items: baseline; }}
  header h1 {{ font-size: 16px; margin: 0; color: #58a6ff; }}
  header span {{ color: #8b949e; font-size: 12px; }}
  main {{ display: grid; grid-template-columns: 340px 1fr; height: calc(100vh - 49px); }}
  #palette {{ border-right: 1px solid #21262d; overflow-y: auto; padding: 8px; }}
  #palette input {{ width: 100%; padding: 8px 10px; background: #161b22; border: 1px solid #30363d;
    border-radius: 6px; color: #c9d1d9; outline: none; margin-bottom: 8px; }}
  .op {{ padding: 8px 10px; border-radius: 6px; cursor: pointer; margin-bottom: 2px; }}
  .op:hover {{ background: #161b22; }}
  .op.selected {{ background: #1f6feb33; }}
  .method {{ display: inline-block; width: 56px; font-weight: 700; font-size: 11px; }}
  .m-GET {{ color: #3fb950; }} .m-POST {{ color: #a371f7; }} .m-PUT {{ color: #d29922; }}
  .m-DELETE {{ color: #f85149; }} .m-PATCH {{ color: #db61a2; }}
  .op .path {{ color: #c9d1d9; }}
  .op .summary {{ display: block; color: #8b949e; font-size: 12px; }}
  #editor {{ padding: 16px 20px; overflow-y: auto; }}
  #editor h2 {{ font-size: 14px; margin: 0 0 12px; color: #e6edf3; }}
  .field {{ margin-bottom: 10px; }}
  .field label {{ display: block; color: #8b949e; font-size: 12px; margin-bottom: 2px; }}
  .field input, .field textarea {{ width: 100%; max-width: 560px; padding: 6px 10px; background: #161b22;
    border: 1px solid #30363d; border-radius: 6px; color: #c9d1d9; font: inherit; }}
  .field textarea {{ min-height: 120px; }}
  #send {{ padding: 8px 20px; background: #238636; color: #fff; border: 0; border-radius: 6px;
    font: inherit; font-weight: 700; cursor: pointer; margin: 12px 0; }}
  #send:hover {{ background: #2ea043; }}
  #response {{ background: #161b22; border: 1px solid #30363d; border-radius: 6px; padding: 12px;
    max-width: 760px; white-space: pre-wrap; word-break: break-all; min-height: 40px; }}
  .status-2 {{ color: #3fb950; }} .status-4 {{ color: #d29922; }} .status-5 {{ color: #f85149; }}
  .empty {{ color: #8b949e; padding: 40px; text-align: center; }}
</style>
</head>
<body>
<header><h1>{title}</h1><span>Contract Playground — requests dispatch through the gateway and are journaled</span></header>
<main>
  <nav id="palette"><input id="filter" placeholder="Filter operations…" autofocus><div id="ops"></div></nav>
  <section id="editor"><div class="empty" id="placeholder">Select an operation to begin</div>
    <div id="form" style="display:none">
      <h2 id="op-title"></h2>
      <div id="path-params"></div>
      <div class="field" id="body-field" style="display:none">
        <label>Request body (JSON)</label>
        <textarea id="body"></textarea>
      </div>
      <button id="send">Send request</button>
      <div id="response">—</div>
    </div>
  </section>
</main>
<script>
const OPS = [{ops_json}];
let selected = null;
const opsEl = document.getElementById('ops');
const filterEl = document.getElementById('filter');

function render(filter = '') {{
  opsEl.innerHTML = '';
  const f = filter.toLowerCase();
  for (const op of OPS) {{
    if (f && !op.id.toLowerCase().includes(f) && !op.path.toLowerCase().includes(f)) continue;
    const div = document.createElement('div');
    div.className = 'op' + (selected === op ? ' selected' : '');
    div.innerHTML = `<span class="method m-${{op.method}}">${{op.method}}</span><span class="path">${{op.path}}</span>` +
      (op.summary ? `<span class="summary">${{op.summary}}</span>` : '');
    div.onclick = () => {{ selected = op; render(filter); showForm(op); }};
    opsEl.appendChild(div);
  }}
}}
filterEl.oninput = () => render(filterEl.value);

function showForm(op) {{
  document.getElementById('placeholder').style.display = 'none';
  document.getElementById('form').style.display = 'block';
  document.getElementById('op-title').textContent = `${{op.method}} ${{op.path}}`;
  const pp = document.getElementById('path-params');
  pp.innerHTML = '';
  for (const p of op.params) {{
    if (p.in !== 'path' && p.in !== 'query') continue;
    const field = document.createElement('div');
    field.className = 'field';
    field.innerHTML = `<label>${{p.name}} (${{p.in}}${{p.required ? ', required' : ''}})</label>` +
      `<input data-param="${{p.name}}" data-in="${{p.in}}" placeholder="${{p.in === 'path' ? p.name : ''}}">`;
    pp.appendChild(field);
  }}
  const bf = document.getElementById('body-field');
  const hasBody = op.method === 'POST' || op.method === 'PUT' || op.method === 'PATCH';
  bf.style.display = hasBody ? 'block' : 'none';
  if (hasBody) document.getElementById('body').value = '{{}}\n';
  document.getElementById('response').textContent = '—';
}}

document.getElementById('send').onclick = async () => {{
  if (!selected) return;
  let path = selected.path;
  const query = new URLSearchParams();
  for (const input of document.querySelectorAll('[data-param]')) {{
    const v = input.value;
    if (!v) continue;
    if (input.dataset.in === 'path') path = path.replace(`{{${{input.dataset.param}}}}`, encodeURIComponent(v));
    else query.set(input.dataset.param, v);
  }}
  const qs = query.toString();
  const url = path + (qs ? '?' + qs : '');
  const opts = {{ method: selected.method, headers: {{}} }};
  const bodyText = document.getElementById('body').value.trim();
  if (bodyText && selected.method !== 'GET' && selected.method !== 'DELETE') {{
    opts.headers['Content-Type'] = 'application/json';
    opts.body = bodyText;
  }}
  const t0 = performance.now();
  try {{
    const resp = await fetch(url, opts);
    const ms = Math.round(performance.now() - t0);
    const text = await resp.text();
    let pretty = text;
    try {{ pretty = JSON.stringify(JSON.parse(text), null, 2); }} catch {{}}
    const cls = resp.status < 300 ? 'status-2' : resp.status < 500 ? 'status-4' : 'status-5';
    document.getElementById('response').innerHTML =
      `<span class="${{cls}}"><b>HTTP ${{resp.status}}</b></span> <span style="color:#8b949e">${{ms}}ms</span>\n${{escapeHtml(pretty)}}`;
  }} catch (e) {{
    document.getElementById('response').innerHTML = `<span class="status-5"><b>Transport error</b></span> ${{escapeHtml(String(e))}}`;
  }}
}};

function escapeHtml(s) {{
  return s.replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');
}}

render();
</script>
</body>
</html>"#
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
