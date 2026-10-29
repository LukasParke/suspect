#!/usr/bin/env python3
"""Draws the README's LSP screenshots from `showcase_data.json`.

Every screenshot shows real server output for `docs/demo/petstore.yaml`
(produced by `cargo run --example showcase -p suspect-lsp`) laid out the
way VS Code's Dark Modern theme presents it: semantic-token colouring,
diagnostic squiggles, the Problems panel, hover / peek / completion /
quick-fix / rename widgets, inlay hints and the symbol quick-pick.

The page is built as HTML (text is escaped per run, so markup can never
leak into the picture), captured with headless Chrome at 2x, and cropped
to its content with Pillow.

    python3 docs/capture/render.py [--out docs/images] [--keep-html]

Requires Google Chrome (or set CHROME=/path/to/chromium) and Pillow.
"""

from __future__ import annotations

import argparse
import html
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DATA_PATH = os.path.join(HERE, "showcase_data.json")
DEFAULT_OUT = os.path.join(HERE, "..", "images")

WIDTH = 1000  # CSS px; captured at 2x
LINE_H = 20
SENTINEL = (255, 0, 255)  # page background below the app; cropped away

CSS = """
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body { background: #ff00ff; }
body { width: %(width)dpx; }
.app {
  width: %(width)dpx; background: #1f1f1f; color: #cccccc;
  font: 13px/1.4 -apple-system, "SF Pro Text", "Segoe UI", system-ui, sans-serif;
  position: relative; overflow: hidden;
}
.mono, .editor, .peek-body, .code { font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace; }
.tabs { background: #181818; height: 35px; display: flex; border-bottom: 1px solid #2b2b2b; }
.tab { padding: 0 14px; height: 35px; display: flex; align-items: center; gap: 8px; background: #1f1f1f;
  border-right: 1px solid #2b2b2b; border-top: 1px solid #0078d4; color: #ffffff; font-size: 13px; }
.tab .yaml { color: #cb4b16; font-weight: 700; font-size: 11px; }
.crumbs { height: 22px; display: flex; align-items: center; padding: 0 16px; color: #a9a9a9; font-size: 12px; gap: 6px; }
.crumbs .sep { color: #6e7681; }
.editor { position: relative; font-size: 13px; line-height: %(line_h)dpx; padding: 4px 0 6px; }
.line { display: flex; height: %(line_h)dpx; white-space: pre; position: relative; }
.line.current { background: rgba(255,255,255,0.035); }
.ln { width: 58px; flex: none; text-align: right; padding-right: 24px; color: #6e7681; }
.line.current .ln { color: #cccccc; }
.lb { width: 18px; flex: none; margin-left: -18px; text-align: center; font-size: 12px; }
.code { flex: 1; min-width: 0; }
.tx { color: #cccccc; }
.key { color: #9cdcfe; }
.str { color: #ce9178; }
.num { color: #b5cea8; }
.pun { color: #cccccc; }
.sem-keyword { color: #569cd6; }
.sem-type { color: #4ec9b0; }
.sem-class { color: #4ec9b0; }
.sem-namespace { color: #4ec9b0; }
.sem-method { color: #dcdcaa; }
.sem-property { color: #9cdcfe; }
.sem-string { color: #ce9178; }
.sem-number { color: #b5cea8; }
.sem-comment { color: #6a9955; }
.sem-enumMember { color: #4fc1ff; }
.sem-parameter { color: #9cdcfe; }
.sem-macro { color: #569cd6; }
.sem-regexp { color: #d16969; }
.outline-sem [class*="sem-"] { outline: 1px dashed rgba(255,255,255,0.28); outline-offset: 1px; border-radius: 2px; }
.sq { text-decoration-line: underline; text-decoration-style: wavy; text-decoration-skip-ink: none; text-underline-offset: 3px; text-decoration-thickness: 1px; }
.sq-error { text-decoration-color: #f14c4c; }
.sq-warning { text-decoration-color: #cca700; }
.sq-info { text-decoration-color: #3794ff; }
.sq-hint { text-decoration-color: #cccccc; text-decoration-style: dotted; }
.link { color: #4daafc; text-decoration: underline; text-underline-offset: 3px; cursor: pointer; }
.sel { background: #264f78; }
.occ { background: rgba(173, 214, 255, 0.15); outline: 1px solid rgba(173, 214, 255, 0.25); }
.caret { display: inline-block; width: 2px; height: 16px; background: #aeafad; vertical-align: text-bottom; margin: 0 -1px; }
.hint { display: inline-block; background: #2b2b2b; color: #969696; border-radius: 3px; padding: 0 4px; font-size: 11.5px; line-height: 16px; vertical-align: 1px; margin-left: 4px; font-style: italic; }
.hint.left { margin-left: 2px; margin-right: 4px; }

.widget { position: absolute; background: #202020; border: 1px solid #454545; box-shadow: 0 2px 8px rgba(0,0,0,0.5); z-index: 5; }
.hover { padding: 8px 12px; max-width: 560px; font-size: 13px; line-height: 1.45; color: #cccccc; }
.hover h3, .hover h4 { font-size: 14px; font-weight: 600; margin-bottom: 6px; color: #ffffff; }
.hover p { margin: 4px 0; }
.hover code { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; background: rgba(255,255,255,0.08); padding: 1px 4px; border-radius: 3px; color: #d7ba7d; }
.hover h3 code, .hover h4 code { color: #4ec9b0; background: none; font-size: 14px; padding: 0; }
.hover em { color: #a9a9a9; }
.hover hr { border: 0; border-top: 1px solid #454545; margin: 8px 0; }
.hover table { border-collapse: collapse; margin: 6px 0 2px; font-size: 12.5px; }
.hover th, .hover td { border: 1px solid #3c3c3c; padding: 3px 10px; text-align: left; }
.hover th { color: #ffffff; font-weight: 600; background: #2a2a2a; }
.hover td:nth-child(3) { text-align: center; color: #89d185; }
.hover ul { padding-left: 18px; }

.menu { min-width: 300px; padding: 4px 0; font-size: 13px; }
.menu .item { display: flex; align-items: center; gap: 8px; padding: 3px 12px; height: 24px; }
.menu .item.active { background: #04395e; color: #ffffff; }
.menu .item .ico { width: 16px; text-align: center; font-size: 12px; }
.menu .item .detail { margin-left: auto; color: #8b949e; font-size: 12px; padding-left: 20px; }
.menu .item.active .detail { color: #c7d2dc; }
.menu .section { color: #8b949e; font-size: 11px; padding: 4px 12px 2px; text-transform: uppercase; letter-spacing: .04em; }
.menu .divider { border-top: 1px solid #3c3c3c; margin: 4px 0; }
.bulb { color: #ffcc00; font-size: 14px; }

.peek { left: 58px; right: 20px; border-left: none; border-right: none; border-top: 1px solid #3794ff; border-bottom: 1px solid #3794ff; box-shadow: none; background: #1f1f1f; }
.peek-head { background: #202b38; height: 28px; display: flex; align-items: center; padding: 0 12px; gap: 10px; font-size: 13px; }
.peek-head .title { color: #ffffff; font-weight: 600; }
.peek-head .meta { color: #a9a9a9; font-size: 12px; }
.peek-head .close { margin-left: auto; color: #a9a9a9; }
.peek-body { font-size: 13px; line-height: %(line_h)dpx; padding: 4px 0; background: #1a1f26; }
.peek-body .ln { color: #5c6370; }
.peek-body .line.current { background: rgba(55,148,255,0.14); }
.peek.flow { position: static; margin: 0 20px 0 58px; }
.editor.top { padding-bottom: 2px; } .editor.bottom { padding-top: 2px; }

.panel { border-top: 1px solid #2b2b2b; background: #181818; }
.panel-tabs { height: 35px; display: flex; align-items: center; padding: 0 16px; gap: 22px; font-size: 11px; letter-spacing: .04em; color: #9d9d9d; text-transform: uppercase; }
.panel-tabs .on { color: #e7e7e7; border-bottom: 1px solid #e7e7e7; padding: 10px 0; }
.panel-tabs .count { display: inline-block; min-width: 18px; padding: 0 5px; border-radius: 9px; background: #4d4d4d; color: #fff; font-size: 11px; text-align: center; margin-left: 6px; letter-spacing: 0; }
.panel-tabs .filter { margin-left: auto; width: 240px; height: 24px; background: #313131; border: 1px solid #3c3c3c; border-radius: 2px; color: #8b949e; text-transform: none; letter-spacing: 0; font-size: 12px; display: flex; align-items: center; padding: 0 8px; }
.problems { padding: 0 0 6px; font-size: 13px; }
.problems .file { display: flex; align-items: center; gap: 8px; height: 22px; padding: 0 16px; color: #cccccc; }
.problems .file .yaml { color: #cb4b16; font-weight: 700; font-size: 11px; }
.problems .file .path { color: #8b949e; font-size: 12px; }
.problems .row { display: flex; align-items: center; gap: 8px; height: 22px; padding: 0 16px 0 38px; white-space: nowrap; overflow: hidden; }
.problems .row.active { background: #04395e; }
.problems .row .msg { overflow: hidden; text-overflow: ellipsis; }
.problems .row .src { color: #8b949e; }
.problems .row .pos { color: #8b949e; margin-left: auto; padding-left: 16px; font-variant-numeric: tabular-nums; }
.problems .more { color: #8b949e; padding: 2px 16px 0 38px; font-size: 12px; }
.ico-error, .ico-warning, .ico-info, .ico-hint { display: inline-block; width: 16px; text-align: center; font-size: 13px; line-height: 16px; }
.ico-error { color: #f14c4c; } .ico-warning { color: #cca700; } .ico-info { color: #3794ff; } .ico-hint { color: #cccccc; }

.status { height: 22px; background: #181818; border-top: 1px solid #2b2b2b; display: flex; align-items: center; padding: 0 10px; font-size: 12px; color: #cccccc; gap: 14px; }
.status .right { margin-left: auto; display: flex; gap: 14px; }
.status .remote { background: #0078d4; color: #fff; height: 22px; display: flex; align-items: center; padding: 0 8px; margin: 0 4px 0 -10px; }

.quick { position: absolute; left: 50%%; top: 6px; transform: translateX(-50%%); width: 600px; background: #222222; border: 1px solid #454545; box-shadow: 0 0 8px 2px rgba(0,0,0,0.5); z-index: 6; padding: 6px; }
.quick .input { height: 26px; background: #313131; border: 1px solid #0078d4; color: #cccccc; display: flex; align-items: center; padding: 0 6px; font-size: 13px; }
.quick .input .ph { color: #8b949e; margin-left: 2px; }
.quick .rows { padding-top: 4px; }
.quick .row { display: flex; align-items: center; height: 22px; padding: 0 6px; gap: 8px; font-size: 13px; border-radius: 3px; }
.quick .row.active { background: #04395e; color: #fff; }
.quick .row .kind { width: 16px; text-align: center; font-size: 12px; }
.quick .row .ctr { color: #8b949e; font-size: 12px; }
.quick .row.active .ctr { color: #c7d2dc; }
.quick .row .file { margin-left: auto; color: #8b949e; font-size: 12px; }
.quick .row.active .file { color: #c7d2dc; }
.k-interface { color: #75beff; } .k-method { color: #b180d7; } .k-struct { color: #ee9d28; } .k-ref { color: #cccccc; }
.dim { opacity: .55; }

.rename { position: absolute; z-index: 6; background: #202020; border: 1px solid #0078d4; box-shadow: 0 2px 8px rgba(0,0,0,0.5); }
.rename .in { height: 24px; min-width: 220px; display: flex; align-items: center; padding: 0 6px; font: 13px ui-monospace, "SF Mono", Menlo, monospace; color: #cccccc; }
.rename .tip { font-size: 11px; color: #8b949e; padding: 0 6px 4px; }
.rename .tip b { color: #cccccc; font-weight: 500; }
.refactor { padding: 0 0 8px; font-size: 13px; }
.refactor .row { display: flex; align-items: center; gap: 8px; height: 22px; padding: 0 16px 0 38px; font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12.5px; white-space: pre; }
.refactor .row .ln { width: 52px; flex: none; color: #8b949e; font-family: -apple-system, "SF Pro Text", system-ui, sans-serif; font-size: 12px; }
.refactor .del { background: rgba(255,0,0,0.18); color: #f48771; text-decoration: line-through; padding: 0 2px; }
.refactor .ins { background: rgba(155,185,85,0.2); color: #b5cea8; padding: 0 2px; }
.refactor .chk { color: #4daafc; }
.legend { position: absolute; right: 20px; top: 10px; z-index: 5; background: #202020; border: 1px solid #454545; box-shadow: 0 2px 8px rgba(0,0,0,0.5); padding: 8px 12px 6px; font-size: 12px; min-width: 210px; }
.legend h4 { font-size: 11px; color: #9d9d9d; text-transform: uppercase; letter-spacing: .04em; margin-bottom: 6px; font-weight: 600; }
.legend .row { display: flex; align-items: center; gap: 8px; height: 20px; }
.legend .sw { width: 10px; height: 10px; border-radius: 2px; }
.legend .n { margin-left: auto; color: #8b949e; font-variant-numeric: tabular-nums; }
.legend .m { font-family: ui-monospace, "SF Mono", Menlo, monospace; font-size: 12px; }
"""

TOKEN_SWATCH = {
    "keyword": "#569cd6",
    "type": "#4ec9b0",
    "class": "#4ec9b0",
    "namespace": "#4ec9b0",
    "method": "#dcdcaa",
    "property": "#9cdcfe",
    "string": "#ce9178",
    "number": "#b5cea8",
    "comment": "#6a9955",
    "enumMember": "#4fc1ff",
    "parameter": "#9cdcfe",
    "macro": "#569cd6",
    "regexp": "#d16969",
}

SEV_ICON = {"error": "⊗", "warning": "⚠", "info": "ⓘ", "hint": "·"}
SYMBOL_ICON = {"Interface": ("◇", "k-interface"), "Method": ("◯", "k-method"), "Struct": ("▣", "k-struct")}


def esc(s: str) -> str:
    return html.escape(s, quote=False)


# ---------------------------------------------------------------------------
# Line rendering: per-character style sets → escaped spans
# ---------------------------------------------------------------------------

KEY_RE = re.compile(r"^(\s*)(-\s+)?([^\s:#][^:#]*?)(:)(\s|$)")
STR_RE = re.compile(r"'[^']*'|\"[^\"]*\"")
NUM_RE = re.compile(r"(?<![\w.])-?\d+(?:\.\d+)?(?![\w.])")


def utf16_to_index(line: str, col: int) -> int:
    """LSP columns count UTF-16 code units; the renderer indexes Python
    strings by code point. Maps one to the other on `line`."""
    units = 0
    for i, ch in enumerate(line):
        if units >= col:
            return i
        units += 2 if ord(ch) > 0xFFFF else 1
    return len(line)


def to_code_points(data: dict, lines: list[str]) -> None:
    """Rewrites every LSP position in `data` (objects with `line` and
    `character`, plus semantic-token `length`s) from UTF-16 code units to
    code-point columns, in place."""
    def visit(node):
        if isinstance(node, dict):
            if "line" in node and "character" in node and isinstance(node["line"], int):
                text = lines[node["line"]] if node["line"] < len(lines) else ""
                start = utf16_to_index(text, node["character"])
                if "length" in node:
                    node["length"] = utf16_to_index(text, node["character"] + node["length"]) - start
                node["character"] = start
            for v in node.values():
                visit(v)
        elif isinstance(node, list):
            for v in node:
                visit(v)

    visit(data)


class Doc:
    """The spec text plus everything that decorates it."""

    def __init__(self, data: dict):
        self.text: str = data["text"]
        self.lines = self.text.split("\n")
        if self.lines and self.lines[-1] == "":
            self.lines.pop()
        to_code_points(data, self.lines)
        self.tokens = data["semantic_tokens"]
        self.base = [self._base_styles(line) for line in self.lines]
        for t in self.tokens:
            row = self.base[t["line"]]
            for c in range(t["character"], min(t["character"] + t["length"], len(row))):
                row[c] = f"sem-{t['type']}"

    @staticmethod
    def _base_styles(line: str) -> list[str]:
        styles = ["tx"] * len(line)
        m = KEY_RE.match(line)
        if m:
            start, end = m.span(3)
            for c in range(start, end):
                styles[c] = "key"
            styles[m.start(4)] = "pun"
        tail_start = m.end(4) if m else 0
        for sm in STR_RE.finditer(line, tail_start):
            for c in range(sm.start(), sm.end()):
                styles[c] = "str"
        for nm in NUM_RE.finditer(line, tail_start):
            if all(styles[c] in ("tx",) for c in range(nm.start(), nm.end())):
                for c in range(nm.start(), nm.end()):
                    styles[c] = "num"
        for c, ch in enumerate(line):
            if ch in "[]{},:" and styles[c] == "tx":
                styles[c] = "pun"
        return styles

    def render_line(self, i: int, decorations: list[tuple[int, int, str]] = (),
                    inserts: dict[int, str] | None = None, caret: int | None = None) -> str:
        """`decorations`: (start_col, end_col, css_class) extra classes.
        `inserts`: {col: raw_html} emitted before that column (inlay hints,
        completion text). `caret`: column that gets a text cursor."""
        line = self.lines[i]
        styles = [{self.base[i][c]} for c in range(len(line))]
        for start, end, cls in decorations:
            for c in range(start, min(end, len(line))):
                styles[c].add(cls)
        out: list[str] = []
        c = 0
        while c < len(line):
            if inserts and c in inserts:
                out.append(inserts[c])
            if caret == c:
                out.append('<span class="caret"></span>')
            j = c
            while j < len(line) and styles[j] == styles[c] and not (inserts and j > c and j in inserts) \
                    and not (caret is not None and j > c and caret == j):
                j += 1
            cls = " ".join(sorted(styles[c]))
            out.append(f'<span class="{cls}">{esc(line[c:j])}</span>')
            c = j
        if inserts and len(line) in inserts:
            out.append(inserts[len(line)])
        if caret == len(line):
            out.append('<span class="caret"></span>')
        return "".join(out)


def editor_html(doc: Doc, first: int, last: int, *, current: int | None = None,
                decorations: dict[int, list[tuple[int, int, str]]] | None = None,
                inserts: dict[int, dict[int, str]] | None = None, caret: tuple[int, int] | None = None,
                gutter: dict[int, str] | None = None, extra: str = "", cls: str = "") -> str:
    """Lines `first..=last` (zero-based) as the editor body."""
    rows = []
    for i in range(first, last + 1):
        line_cls = "line current" if i == current else "line"
        bullet = f'<span class="lb">{gutter[i]}</span>' if gutter and i in gutter else ""
        code = doc.render_line(
            i,
            (decorations or {}).get(i, []),
            (inserts or {}).get(i),
            caret[1] if caret and caret[0] == i else None,
        )
        rows.append(f'<div class="{line_cls}" data-line="{i}"><span class="ln">{i + 1}</span>{bullet}<span class="code">{code}</span></div>')
    return f'<div class="editor {cls}">{"".join(rows)}{extra}</div>'


def line_top(first: int, i: int) -> int:
    """Pixel offset of line `i` inside an editor that starts at `first`."""
    return 4 + (i - first) * LINE_H


def col_x(col: int) -> int:
    """Approximate x of a column: gutter + col * glyph width (SF Mono 13px ≈ 7.8px)."""
    return 58 + round(col * 7.83)


# ---------------------------------------------------------------------------
# Markdown (the hover's dialect: headings, emphasis, code, tables, hr)
# ---------------------------------------------------------------------------

def inline_md(s: str) -> str:
    s = esc(s)
    s = re.sub(r"`([^`]+)`", r"<code>\1</code>", s)
    s = re.sub(r"\*\*([^*]+)\*\*", r"<strong>\1</strong>", s)
    s = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"<em>\1</em>", s)
    return s


def markdown_html(md: str) -> str:
    out: list[str] = []
    lines = md.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        if not line.strip():
            i += 1
            continue
        if line.startswith("|"):
            rows = []
            while i < len(lines) and lines[i].startswith("|"):
                cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
                if not all(re.fullmatch(r":?-{3,}:?", c) for c in cells):
                    rows.append(cells)
                i += 1
            head, body = rows[0], rows[1:]
            out.append("<table><thead><tr>" + "".join(f"<th>{inline_md(c)}</th>" for c in head) + "</tr></thead><tbody>")
            for r in body:
                out.append("<tr>" + "".join(f"<td>{inline_md(c)}</td>" for c in r) + "</tr>")
            out.append("</tbody></table>")
            continue
        m = re.match(r"^(#{1,6})\s+(.*)$", line)
        if m:
            level = min(len(m.group(1)), 3) + 1
            out.append(f"<h{level}>{inline_md(m.group(2))}</h{level}>")
        elif re.fullmatch(r"-{3,}", line.strip()):
            out.append("<hr>")
        elif line.lstrip().startswith("- "):
            items = []
            while i < len(lines) and lines[i].lstrip().startswith("- "):
                items.append(f"<li>{inline_md(lines[i].lstrip()[2:])}</li>")
                i += 1
            out.append("<ul>" + "".join(items) + "</ul>")
            continue
        else:
            out.append(f"<p>{inline_md(line)}</p>")
        i += 1
    return "".join(out)


# ---------------------------------------------------------------------------
# Window chrome
# ---------------------------------------------------------------------------

def tabs_html(name: str) -> str:
    return (f'<div class="tabs"><div class="tab"><span class="yaml">YAML</span>{esc(name)}</div></div>'
            f'<div class="crumbs">docs <span class="sep">›</span> demo <span class="sep">›</span> {esc(name)}</div>')


def status_html(diags: list[dict], line: int, col: int, extra_left: str = "") -> str:
    errors = sum(d["severity"] == "error" for d in diags)
    warnings = sum(d["severity"] == "warning" for d in diags)
    infos = sum(d["severity"] in ("info", "hint") for d in diags)
    return (
        '<div class="status"><span class="remote">⚡ suspect</span>'
        f'<span><span class="ico-error">⊗</span> {errors} <span class="ico-warning">⚠</span> {warnings} '
        f'<span class="ico-info">ⓘ</span> {infos}</span>{extra_left}'
        f'<span class="right"><span>Ln {line + 1}, Col {col + 1}</span><span>Spaces: 2</span><span>UTF-8</span><span>YAML</span></span></div>'
    )


def page(body: str) -> str:
    return f'<!doctype html><html><head><meta charset="utf-8"><style>{CSS % {"width": WIDTH, "line_h": LINE_H}}</style></head><body><div class="app">{body}</div></body></html>'


def sev_decorations(diags: list[dict], doc: Doc) -> dict[int, list[tuple[int, int, str]]]:
    """Squiggles per line. A diagnostic spanning several lines underlines
    its first line only (its anchor), as VS Code's multi-line squiggle is
    unreadable at this size."""
    rank = {"error": 3, "warning": 2, "info": 1, "hint": 0}
    best: dict[tuple[int, int, int], str] = {}
    for d in diags:
        s, e = d["range"]["start"], d["range"]["end"]
        start_col = s["character"]
        end_col = e["character"] if e["line"] == s["line"] else len(doc.lines[s["line"]])
        key = (s["line"], start_col, end_col)
        if key not in best or rank[d["severity"]] > rank[best[key]]:
            best[key] = d["severity"]
    out: dict[int, list[tuple[int, int, str]]] = {}
    for (line, a, b), sev in best.items():
        out.setdefault(line, []).extend([(a, b, "sq"), (a, b, f"sq-{sev}")])
    return out


# ---------------------------------------------------------------------------
# Feature pages
# ---------------------------------------------------------------------------

def page_diagnostics(data: dict, doc: Doc) -> str:
    diags = data["diagnostics"]
    first, last = 0, 25
    # The cursor sits on the undeclared path parameter, the one semantic
    # error in the file; its Problems row is the selected one.
    selected = next((d for d in diags if d["code"] == "oas-path-param-not-declared"), diags[0])
    current = selected["range"]["start"]["line"]
    body = tabs_html(data["file"])
    body += editor_html(doc, first, last, current=current, decorations=sev_decorations(diags, doc))
    rows = []
    shown = 0
    for d in diags:
        if d["source"] == "suspect-lint" and d["severity"] == "info":
            continue  # keep the panel to one screen: the lint infos are repetitive
        s = d["range"]["start"]
        active = " active" if d is selected else ""
        rows.append(
            f'<div class="row{active}"><span class="ico-{d["severity"]}">{SEV_ICON[d["severity"]]}</span>'
            f'<span class="msg">{esc(d["message"])}</span>'
            f'<span class="src">{esc(d["source"])}({esc(d["code"])})</span>'
            f'<span class="pos">[Ln {s["line"] + 1}, Col {s["character"] + 1}]</span></div>'
        )
        shown += 1
    body += (
        '<div class="panel"><div class="panel-tabs"><span class="on">Problems'
        f'<span class="count">{len(diags)}</span></span><span>Output</span><span>Terminal</span>'
        '<span class="filter">Filter (e.g. text, **/*.yaml)</span></div>'
        f'<div class="problems"><div class="file"><span class="yaml">YAML</span>{esc(data["file"])}'
        f'<span class="path">docs/demo</span><span class="count" style="background:#4d4d4d;color:#fff;border-radius:9px;padding:0 5px;font-size:11px">{len(diags)}</span></div>'
        + "".join(rows)
        + (f'<div class="more">… {len(diags) - shown} more</div>' if shown < len(diags) else "")
        + "</div></div>"
    )
    body += status_html(diags, current, selected["range"]["start"]["character"])
    return page(body)


def page_hover(data: dict, doc: Doc) -> str:
    h = data["hover"]
    r = h["range"]
    line = r["start"]["line"]
    first, last = 14, 38  # enough lines below the $ref for the hover card
    deco = {line: [(r["start"]["character"], r["end"]["character"], "occ")]}
    top = line_top(first, line) + LINE_H + 2
    left = col_x(r["start"]["character"]) - 2
    widget = f'<div class="widget hover" style="left:{left}px;top:{top}px">{markdown_html(h["markdown"])}</div>'
    body = tabs_html(data["file"])
    body += editor_html(doc, first, last, current=line, decorations=deco, extra=widget)
    body += status_html(data["diagnostics"], line, r["start"]["character"] + 3)
    return page(body)


def page_code_actions(data: dict, doc: Doc) -> str:
    ca = data["code_actions"]
    r = ca["range"]
    line = r["start"]["line"]
    first, last = 4, 16
    deco = sev_decorations(data["diagnostics"], doc)
    top = line_top(first, line) + LINE_H + 2
    left = col_x(r["start"]["character"])
    items = []
    for i, a in enumerate(ca["actions"]):
        if a["kind"] and a["kind"].startswith("source."):
            items.append('<div class="divider"></div><div class="section">Source action</div>')
        ico = "🛠" if a["kind"] and a["kind"].startswith("source.") else "💡"
        items.append(f'<div class="item{" active" if i == 0 else ""}"><span class="ico">{ico}</span>{esc(a["title"])}</div>')
    widget = f'<div class="widget menu" style="left:{left}px;top:{top}px"><div class="section">Quick fix</div>{"".join(items)}</div>'
    body = tabs_html(data["file"])
    body += editor_html(doc, first, last, current=line, decorations=deco, extra=widget,
                        gutter={line: '<span class="bulb">💡</span>'},
                        caret=(line, r["start"]["character"]))
    body += status_html(data["diagnostics"], line, r["start"]["character"])
    return page(body)


def page_completion(data: dict, doc: Doc) -> str:
    c = data["completion"]
    p = c["position"]
    line = p["line"]
    first, last = 8, 19
    # Show the line as the author is typing it: everything before the
    # cursor plus the characters typed so far, then the caret.
    typed = c["typed"]
    full = doc.lines[line]
    prefix = full[: p["character"]]
    shown = Doc({"text": prefix + typed + "'\n", "semantic_tokens": []})
    shown.base[0] = doc.base[line][: len(prefix)] + ["str"] * (len(typed) + 1)
    shown_html = shown.render_line(0, caret=len(prefix) + len(typed))
    items = []
    for i, label in enumerate(c["items"]):
        items.append(
            f'<div class="item{" active" if i == 0 else ""}"><span class="ico k-ref">⇢</span>'
            f'<span class="mono">{esc(label)}</span><span class="detail">components/schemas</span></div>'
        )
    top = line_top(first, line) + LINE_H + 2
    left = col_x(p["character"]) - 2
    widget = f'<div class="widget menu" style="left:{left}px;top:{top}px;min-width:420px">{"".join(items)}</div>'
    rows = []
    for i in range(first, last + 1):
        if i == line:
            code = shown_html
        else:
            code = doc.render_line(i)
        cls = "line current" if i == line else "line"
        rows.append(f'<div class="{cls}"><span class="ln">{i + 1}</span><span class="code">{code}</span></div>')
    body = tabs_html(data["file"]) + f'<div class="editor">{"".join(rows)}{widget}</div>'
    body += status_html(data["diagnostics"], line, p["character"] + len(typed))
    return page(body)


def page_goto_def(data: dict, doc: Doc) -> str:
    g = data["goto_def"]
    s, t = g["source"], g["target"]
    line = s["start"]["line"]
    first, last = 8, 16
    deco = {line: [(s["start"]["character"], s["end"]["character"], "link")]}
    # The definition range covers the component's value mapping; the peek
    # also shows the key line above it (`Pets:`), which names the target.
    v_first = t["start"]["line"]
    v_last = t["end"]["line"]
    if t["end"]["character"] == 0 and v_last > v_first:
        v_last -= 1  # an exclusive end at column 0 does not include that line
    v_last = min(v_last, len(doc.lines) - 1)
    indent = len(doc.lines[v_first]) - len(doc.lines[v_first].lstrip())
    key_line = v_first
    while key_line > 0 and (len(doc.lines[key_line]) - len(doc.lines[key_line].lstrip())) >= indent:
        key_line -= 1
    peek_rows = []
    for i in range(key_line, v_last + 1):
        cls = "line current" if v_first <= i <= v_last else "line"
        peek_rows.append(f'<div class="{cls}"><span class="ln">{i + 1}</span><span class="code">{doc.render_line(i)}</span></div>')
    name = doc.lines[key_line].strip().rstrip(":")
    peek = (
        f'<div class="widget peek flow"><div class="peek-head"><span class="title">{esc(data["file"])}</span>'
        f'<span class="meta">components › schemas › {esc(name)} — Ln {key_line + 1}</span><span class="close">✕</span></div>'
        f'<div class="peek-body">{"".join(peek_rows)}</div></div>'
    )
    # The peek opens inline under the reference and pushes the rest down.
    body = tabs_html(data["file"])
    body += editor_html(doc, first, line, current=line, decorations=deco, cls="top")
    body += peek
    body += editor_html(doc, line + 1, last, cls="bottom")
    body += status_html(data["diagnostics"], line, s["start"]["character"] + 3)
    return page(body)


def page_semantic(data: dict, doc: Doc) -> str:
    counts: dict[str, int] = {}
    for t in data["semantic_tokens"]:
        counts[t["type"]] = counts.get(t["type"], 0) + 1
    rows = "".join(
        f'<div class="row"><span class="sw" style="background:{TOKEN_SWATCH.get(k, "#ccc")}"></span>'
        f'<span class="m">{esc(k)}</span><span class="n">{n}</span></div>'
        for k, n in sorted(counts.items(), key=lambda kv: -kv[1])
    )
    legend = f'<div class="legend"><h4>Semantic tokens · {len(data["semantic_tokens"])}</h4>{rows}</div>'
    body = tabs_html(data["file"])
    body += editor_html(doc, 0, len(doc.lines) - 1, extra=legend, cls="outline-sem")
    body += status_html(data["diagnostics"], 0, 0)
    return page(body)


def page_inlay(data: dict, doc: Doc) -> str:
    inserts: dict[int, dict[int, str]] = {}
    for h in data["inlay_hints"]:
        p = h["position"]
        cls = "hint left" if h["padding_left"] else "hint"
        inserts.setdefault(p["line"], {})[p["character"]] = f'<span class="{cls}">{esc(h["label"])}</span>'
    first = max(0, min(inserts) - 1)
    last = len(doc.lines) - 1
    body = tabs_html(data["file"])
    body += editor_html(doc, first, last, inserts=inserts)
    body += status_html(data["diagnostics"], last, 0)
    return page(body)


def page_symbols(data: dict, doc: Doc) -> str:
    rows = []
    for i, s in enumerate(data["workspace_symbols"]):
        ico, cls = SYMBOL_ICON.get(s["kind"], ("◌", "k-ref"))
        ctr = f'<span class="ctr">{esc(s["container"])}</span>' if s.get("container") else ""
        rows.append(
            f'<div class="row{" active" if i == 0 else ""}"><span class="kind {cls}">{ico}</span>'
            f'<span class="mono">{esc(s["name"])}</span>{ctr}<span class="file">{esc(data["file"])} · Ln {s["line"] + 1}</span></div>'
        )
    quick = (
        '<div class="quick"><div class="input"><span class="mono">#</span>'
        '<span class="caret"></span><span class="ph">Go to Symbol in Workspace</span></div>'
        f'<div class="rows">{"".join(rows)}</div></div>'
    )
    body = tabs_html(data["file"])
    body += f'<div style="position:relative">{quick}' + editor_html(doc, 0, 15, cls="dim") + "</div>"
    body += status_html(data["diagnostics"], 0, 0)
    return page(body)


def page_rename(data: dict, doc: Doc) -> str:
    rn = data["rename"]
    r = rn["range"]
    line = r["start"]["line"]
    first, last = 24, len(doc.lines) - 1
    deco: dict[int, list[tuple[int, int, str]]] = {}
    for e in rn["edits"]:
        er = e["range"]
        deco.setdefault(er["start"]["line"], []).append((er["start"]["character"], er["end"]["character"], "occ"))
    deco[line] = [(r["start"]["character"], r["end"]["character"], "sel")]
    top = line_top(first, line) - 2
    left = col_x(r["start"]["character"]) - 1
    box = (
        f'<div class="rename" style="left:{left}px;top:{top}px"><div class="in">{esc(rn["new_name"])}<span class="caret"></span></div>'
        '<div class="tip">Press <b>Enter</b> to Rename, <b>Shift+Enter</b> to Preview</div></div>'
    )
    rows = []
    for e in rn["edits"]:
        er = e["range"]
        src = doc.lines[er["start"]["line"]]
        a, b = er["start"]["character"], er["end"]["character"]
        rows.append(
            f'<div class="row"><span class="chk">☑</span><span class="ln">Ln {er["start"]["line"] + 1}</span>'
            f'<span class="tx">{esc(src[:a].lstrip())}</span><span class="del">{esc(src[a:b])}</span>'
            f'<span class="ins">{esc(e["new_text"])}</span><span class="tx">{esc(src[b:])}</span></div>'
        )
    panel = (
        '<div class="panel"><div class="panel-tabs"><span class="on">Refactor Preview'
        f'<span class="count">{len(rn["edits"])}</span></span><span>Problems</span><span>Output</span></div>'
        f'<div class="refactor"><div class="file" style="display:flex;gap:8px;align-items:center;height:22px;padding:0 16px">'
        f'<span class="chk">☑</span><span class="yaml" style="color:#cb4b16;font-weight:700;font-size:11px">YAML</span>{esc(data["file"])}'
        f'<span style="color:#8b949e;font-size:12px">rename {esc(rn["placeholder"])} → {esc(rn["new_name"])}</span></div>{"".join(rows)}</div></div>'
    )
    body = tabs_html(data["file"])
    body += editor_html(doc, first, last, current=line, decorations=deco, extra=box)
    body += panel
    body += status_html(data["diagnostics"], line, r["start"]["character"])
    return page(body)


PAGES = {
    "diagnostics": page_diagnostics,
    "hover": page_hover,
    "code_actions": page_code_actions,
    "completion": page_completion,
    "goto_def": page_goto_def,
    "semantic_tokens": page_semantic,
    "inlay_hints": page_inlay,
    "workspace_symbols": page_symbols,
    "rename": page_rename,
}


# ---------------------------------------------------------------------------
# Capture
# ---------------------------------------------------------------------------

def find_chrome() -> str:
    env = os.environ.get("CHROME")
    if env:
        return env
    candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ]
    for c in candidates:
        if os.path.isabs(c) and os.path.exists(c):
            return c
        if not os.path.isabs(c) and shutil.which(c):
            return shutil.which(c)  # type: ignore[return-value]
    sys.exit("Chrome not found; set CHROME=/path/to/chrome")


def capture(chrome: str, html_path: str, png_path: str, scale: int = 2) -> None:
    from PIL import Image

    tall = 1800
    subprocess.run(
        [
            chrome, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
            "--disable-extensions", "--font-render-hinting=none",
            f"--force-device-scale-factor={scale}", f"--window-size={WIDTH},{tall}",
            f"--screenshot={png_path}", pathlib.Path(html_path).absolute().as_uri(),
        ],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120,
    )
    img = Image.open(png_path).convert("RGB")
    w, h = img.size
    px = img.load()
    bottom = h
    for y in range(h - 1, -1, -1):
        if px[0, y] != SENTINEL or px[w - 1, y] != SENTINEL:
            bottom = y + 1
            break
    else:
        bottom = h
    if bottom >= h:
        sys.exit(f"{os.path.basename(html_path)} is taller than {tall}px; raise `tall` in capture()")
    img.crop((0, 0, w, bottom)).save(png_path, optimize=True)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=DEFAULT_OUT, help="directory for lsp-*.png")
    ap.add_argument("--only", nargs="+", choices=sorted(PAGES), help="render a subset")
    ap.add_argument("--keep-html", action="store_true", help="leave the HTML next to the PNGs")
    args = ap.parse_args()

    with open(DATA_PATH, encoding="utf-8") as f:
        data = json.load(f)
    doc = Doc(data)
    chrome = find_chrome()
    os.makedirs(args.out, exist_ok=True)
    work = args.out if args.keep_html else tempfile.mkdtemp(prefix="suspect-showcase-")
    try:
        for name, build in PAGES.items():
            if args.only and name not in args.only:
                continue
            html_path = os.path.join(work, f"lsp-{name}.html")
            png_path = os.path.join(args.out, f"lsp-{name}.png")
            with open(html_path, "w", encoding="utf-8") as f:
                f.write(build(data, doc))
            capture(chrome, html_path, png_path)
            print(f"wrote {os.path.relpath(png_path)}")
    finally:
        if not args.keep_html:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
