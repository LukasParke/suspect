#!/usr/bin/env python3
"""Generates VS Code-styled HTML pages showcasing suspect LSP features from
showcase_data.json. Each page renders actual LSP server output as it would
appear in VS Code Dark+ theme."""

import json
import os
import sys

DATA_PATH = os.path.join(os.path.dirname(__file__), "showcase_data.json")
OUT_DIR = os.path.join(os.path.dirname(__file__), "..", "images")

BASE_CSS = """
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  background: #1e1e1e;
  color: #d4d4d4;
  font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', Menlo, Consolas, monospace;
  font-size: 14px;
  padding: 0;
  line-height: 1.6;
}
.titlebar {
  background: #323233;
  padding: 8px 16px;
  font-size: 12px;
  color: #cccccc;
  border-bottom: 1px solid #454545;
}
.titlebar .filename { color: #e8e8e8; font-weight: 500; }
.editor {
  padding: 12px 20px;
  min-height: 200px;
  position: relative;
}
.line { display: flex; white-space: pre; }
.line-num {
  width: 40px; text-align: right; padding-right: 16px;
  color: #6e7681; user-select: none; flex-shrink: 0;
}
.line-content { flex: 1; }
.kw { color: #c586c0; }
.type { color: #4ec9b0; }
.str { color: #ce9178; }
.num { color: #b5cea8; }
.prop { color: #9cdcfe; }
.fn { color: #dcdcaa; }
.comment { color: #6a9955; font-style: italic; }
.squiggle-error { text-decoration: underline wavy #f14c4c; text-underline-offset: 3px; }
.squiggle-warning { text-decoration: underline wavy #cca700; text-underline-offset: 3px; }
.hover-card {
  background: #252526;
  border: 1px solid #454545;
  border-radius: 4px;
  padding: 12px 16px;
  max-width: 600px;
  box-shadow: 0 4px 12px rgba(0,0,0,0.4);
  margin: 4px 0 0 60px;
  font-size: 13px;
}
.hover-card table {
  border-collapse: collapse; width: 100%; margin: 8px 0;
}
.hover-card th, .hover-card td {
  border: 1px solid #454545; padding: 4px 10px; text-align: left;
}
.hover-card th { background: #2d2d2d; color: #cccccc; }
.completion-menu {
  background: #252526;
  border: 1px solid #454545;
  border-radius: 3px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.5);
  margin-left: 60px;
  width: 400px;
}
.completion-item {
  padding: 4px 12px;
  display: flex;
  align-items: center;
  gap: 8px;
}
.completion-item.selected { background: #04395e; }
.completion-icon {
  width: 16px; height: 16px; border-radius: 3px;
  display: inline-flex; align-items: center; justify-content: center;
  font-size: 11px; font-weight: bold; flex-shrink: 0;
}
.ci-field { background: #4fc1ff22; color: #4fc1ff; }
.ci-var { background: #4ec9b022; color: #4ec9b0; }
.ci-snippet { background: #c586c022; color: #c586c0; }
.inlay-hint {
  color: #888888;
  font-style: italic;
  font-size: 13px;
}
.feature-label {
  background: #007acc;
  color: white;
  padding: 4px 12px;
  font-size: 12px;
  font-family: -apple-system, sans-serif;
  display: inline-block;
  margin-bottom: 8px;
  border-radius: 0 4px 4px 0;
}
.problems-panel {
  background: #1e1e1e;
  border-top: 1px solid #454545;
  padding: 8px 16px;
  max-height: 250px;
  overflow-y: auto;
}
.problem-row {
  padding: 2px 0;
  font-size: 13px;
  display: flex; gap: 8px; align-items: baseline;
}
.sev-error { color: #f14c4c; }
.sev-warning { color: #cca700; }
.sev-info { color: #3794ff; }
.diag-code { color: #888; font-size: 12px; }
"""

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

def yaml_line(num, content, decorations=None):
    dec = ""
    if decorations:
        for pattern, css_class in decorations.items():
            if pattern in content:
                content = content.replace(pattern, f'<span class="{css_class}">{pattern}</span>')
    highlighted = content
    import re
    # Simple YAML highlighting
    highlighted = re.sub(r'^(\s*)(\w[\w./{}$-]*)(:)', r'\1<span class="prop">\2</span><span class="kw">:</span>', highlighted)
    highlighted = re.sub(r"'([^']*)'", r"<span class=\"str\">'\1'</span>", highlighted)
    highlighted = re.sub(r'"([^"]*)"', r'<span class="str">"\1"</span>', highlighted)
    highlighted = re.sub(r'\b(true|false|null)\b', r'<span class="kw">\1</span>', highlighted)
    highlighted = re.sub(r'\b(\d+)\b', r'<span class="num">\1</span>', highlighted)
    return f'<div class="line"><span class="line-num">{num}</span><span class="line-content">{highlighted}{dec}</span></div>'

def make_page(title, feature_label, body_html, lines_html="", extra_css=""):
    return f"""<!DOCTYPE html>
<html><head><style>{BASE_CSS}{extra_css}</style></head>
<body>
<div class="titlebar"><span class="filename">petstore.yaml — suspect</span></div>
<div style="padding: 12px 16px;"><span class="feature-label">{feature_label}</span></div>
{body_html}
</body></html>"""


def main():
    data = json.load(open(DATA_PATH))
    out_dir = OUT_DIR
    os.makedirs(out_dir, exist_ok=True)

    spec_lines = open(os.path.join(os.path.dirname(DATA_PATH), "..", "..", "docs", "demo", "petstore.yaml")).readlines()

    def render_lines(start=1, count=30, decorations=None):
        html = '<div class="editor">'
        end = min(start + count, len(spec_lines) + 1)
        for i in range(start, end):
            content = spec_lines[i - 1].rstrip("\n") if i <= len(spec_lines) else ""
            decs = {}
            if decorations:
                for d in decorations:
                    if d.get("line") == i:
                        decs[d["text"]] = d["css"]
            html += yaml_line(i, content, decs)
        html += "</div>"
        return html

    for entry in data:
        feat = entry["feature"]
        title = entry["title"]
        body = ""

        if feat == "hover":
            body = render_lines(1, 15)
            md = entry["markdown"].replace("\n", "<br>").replace("**", "<b>")
            body += f'<div class="hover-card">{md}</div>'
            html = make_page(feat, title, body)
            fname = f"lsp-{feat}.html"

        elif feat == "diagnostics":
            all_diags = []
            for d in entry.get("validate", []):
                sev_class = {"Error": "sev-error", "Warning": "sev-warning"}.get(d["severity"], "sev-info")
                line_num = d["start"] // 40 + 1
                all_diags.append({
                    "line": line_num,
                    "text": d["message"][:50],
                    "css": f"squiggle-{('error' if 'Error' in d['severity'] else 'warning')}",
                })
            decs = [{"line": d["line"], "text": "", "css": ""} for d in all_diags[:5]]
            body = render_lines(1, len(spec_lines), decs)
            body += '<div class="problems-panel">'
            for d in entry.get("validate", [])[:6]:
                sev = d["severity"].lower().replace("error", "error")
                cls = {"Error": "sev-error", "Warning": "sev-warning"}.get(d["severity"], "sev-info")
                line_n = d["start"] // 40 + 1
                body += f'<div class="problem-row"><span class="{cls}">⨯</span> <span>{esc(d["message"])}</span> <span class="diag-code">{esc(d["code"])}</span> <span style="color:#888">[line {line_n}]</span></div>'
            for l in entry.get("lint", [])[:4]:
                body += f'<div class="problem-row"><span class="sev-warning">⚠</span> <span>{esc(l["message"])}</span> <span class="diag-code">{esc(l["code"])}</span></div>'
            body += "</div>"
            html = make_page(feat, title, body)
            fname = f"lsp-{feat}.html"

        elif feat == "code_actions":
            actions = entry.get("actions", [])
            body = render_lines(1, 8)
            if actions:
                body += '<div class="completion-menu">'
                for a in actions:
                    body += f'<div class="completion-item selected"><span class="completion-icon ci-snippet">💡</span> {esc(a["title"])}</div>'
                body += "</div>"
            else:
                body += '<div style="color:#888;padding:8px 60px;">No quick fixes available at this position.</div>'
            html = make_page(feat, title, body)
            fname = f"lsp-{feat}.html"

        elif feat == "inlay_hints":
            hints = entry.get("hints", [])
            hint_positions = {}
            for h in hints:
                try:
                    label_data = h["label"]
                    line = h["line"]
                    if isinstance(label_data, str) and "String(" in label_data:
                        label_text = label_data.split('"')[1] if '"' in label_data else ""
                    else:
                        label_text = str(label_data)
                    hint_positions[line] = label_text
                except Exception:
                    pass
            body = render_lines(1, min(len(spec_lines), 42))
            # add inlay hint annotations
            extra_css = ".inlay-inline { color: #888; font-style: italic; font-size: 13px; }"
            html = make_page(feat, title, body, extra_css)
            fname = f"lsp-{feat}.html"

        elif feat == "workspace_symbols":
            symbols = entry.get("names", [])
            body = '<div style="padding:12px;">'
            body += '<div class="completion-menu" style="width:500px;">'
            body += '<div class="completion-item selected"><span style="color:#888;margin-right:8px;">🔍</span> <input type="text" placeholder="Search symbols..." style="background:transparent;border:none;color:#ccc;width:100%;font-family:inherit;font-size:14px;" value=""></div>'
            icon_map = {"STRUCT": ("S", "ci-var"), "METHOD": ("M", "ci-field"), "FUNCTION": ("ƒ", "ci-field"), "INTERFACE": ("I", "ci-field"), "CONSTANT": ("C", "ci-var")}
            for name in symbols[:15]:
                kind_letter, icon_cls = icon_map.get(name.split()[-1] if " " in name else name.split("/")[0] if "/" in name else name, ("◇", "ci-field"))
                body += f'<div class="completion-item"><span class="completion-icon {icon_cls}">{kind_letter}</span> {esc(name)}</div>'
            body += "</div></div>"
            html = make_page(feat, title, body)
            fname = f"lsp-{feat}.html"

        else:
            # generic fallback
            body = f'<div class="editor"><pre>{json.dumps(entry, indent=2)[:2000]}</pre></div>'
            html = make_page(feat, title, body)
            fname = f"lsp-{feat}.html"

        path = os.path.join(out_dir, f"{fname}")
        with open(path, "w") as f:
            f.write(html)
        print(f"generated: {path}")

    print(f"Generated {len(data)} HTML pages in {out_dir}")


if __name__ == "__main__":
    main()
