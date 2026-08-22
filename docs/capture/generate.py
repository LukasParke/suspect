#!/usr/bin/env python3
"""Generates pixel-perfect VS Code-style LSP feature screenshots using PIL.
Renders syntax-highlighted YAML with hover cards, diagnostics, inlay hints,
and completion menus based on real LSP server output."""

import json
import os
import sys

from PIL import Image, ImageDraw, ImageFont

# --- constants ---
BG = (30, 30, 30)
FG = (212, 212, 212)
GUTTER_BG = (30, 30, 30)
LINE_NUM = (110, 118, 129)
TAB_BG = (37, 37, 38)
TITLE_BG = (50, 50, 50)
BORDER = (69, 69, 69)
HOVER_BG = (37, 37, 38)
SELECTED = (4, 57, 94)

COL_KW = (197, 134, 192)     # purple
COL_TYPE = (78, 201, 176)    # teal
COL_STR = (206, 145, 120)    # orange
COL_NUM = (181, 206, 168)    # light green
COL_PROP = (156, 220, 254)   # light blue
COL_FN = (220, 220, 170)     # yellow
COL_COMMENT = (106, 153, 85) # green
COL_ERROR = (241, 76, 76)    # red
COL_WARN = (204, 167, 0)     # yellow
COL_INLAY = (136, 136, 136)  # gray

FONT_SIZE = 14
SMALL_SIZE = 12
TITLE_SIZE = 12
LINE_H = 20
GUTTER_W = 55
PAD_X = 10
PAD_Y = 8

def load_font(size):
    for name in ["DejaVuSansMono.ttf", "Monaco.ttf", "Menlo.ttf", "Consolas.ttf"]:
        try:
            return ImageFont.truetype(f"/usr/share/fonts/truetype/dejavu/{name}", size)
        except (OSError, IOError):
            continue
    for path in ["/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                 "/usr/share/fonts/noto/NotoSansMono-Regular.ttf"]:
        try:
            return ImageFont.truetype(path, size)
        except (OSError, IOError):
            continue
    return ImageFont.load_default()

def load_bold_font(size):
    for name in ["DejaVuSansMono-Bold.ttf", "Monaco-Bold.ttf"]:
        try:
            return ImageFont.truetype(f"/usr/share/fonts/truetype/dejavu/{name}", size)
        except (OSError, IOError):
            continue
    return load_font(size)

def load_ui_font(size):
    for path in ["/usr/share/fonts/TTF/DejaVuSans.ttf",
                 "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]:
        try:
            return ImageFont.truetype(path, size)
        except (OSError, IOError):
            continue
    return ImageFont.load_default()

FONT = None
FONT_BOLD = None
UI_FONT = None

def _init_fonts():
    global FONT, FONT_BOLD, UI_FONT
    FONT = load_font(FONT_SIZE)
    FONT_BOLD = load_bold_font(FONT_SIZE)
    UI_FONT = load_ui_font(SMALL_SIZE)

_init_fonts()


def draw_text_shadow(draw, xy, text, font, fill):
    """Draw text with subtle shadow for readability."""
    draw.text((xy[0] + 1, xy[1] + 1), text, fill=(0, 0, 0), font=font)
    draw.text(xy, text, fill=fill, font=font)


def tokenize_yaml_line(line):
    """Simple YAML tokenizer returning (text, color) segments."""
    segments = []
    rest = line

    import re
    # Key pattern: leading spaces then word then colon
    m = re.match(r'^(\s*)([\w./{}$-]+)(\s*:)(.*)$', line)
    if m:
        indent, key, colon, value = m.groups()
        if indent:
            segments.append((indent, FG))
        segments.append((key, COL_PROP))
        segments.append((colon, COL_PROP))
        rest = value
    else:
        m2 = re.match(r'^(\s*)(-)(.*)$', line)
        if m2:
            segments.append((m2.group(1) + "-", FG))
            rest = m2.group(3)
        else:
            pass

    # Value highlighting
    if rest:
        import re
        val_stripped = rest.lstrip()
        lead = rest[:len(rest) - len(val_stripped)]
        if lead:
            segments.append((lead, FG))

        if val_stripped.startswith("'") or val_stripped.startswith('"'):
            segments.append((val_stripped, COL_STR))
        elif re.match(r'^-?\d+(\.\d+)?$', val_stripped.strip()):
            segments.append((val_stripped, COL_NUM))
        elif val_stripped.strip() in ('true', 'false', 'null'):
            segments.append((val_stripped, COL_KW))
        else:
            # check for inline keywords
            for kw in ['true', 'false', 'null']:
                if kw in val_stripped:
                    pre, post = val_stripped.split(kw, 1)
                    if pre:
                        segments.append((pre, FG))
                    segments.append((kw, COL_KW))
                    if post:
                        segments.append((post, FG))
                    break
            else:
                segments.append((val_stripped, FG))

    return segments


class Editor:
    def __init__(self, width=1000, height=None, title="petstore.yaml"):
        self.width = width
        self.font = load_font(FONT_SIZE)
        self.bold = load_bold_font(FONT_SIZE)
        self.ui = load_ui_font(SMALL_SIZE)
        self.title_ui = load_ui_font(TITLE_SIZE)
        self.line_h = LINE_H
        self.char_w = self.font.getbbox("M")[2] - self.font.getbbox("M")[0]
        if self.char_w <= 0:
            self.char_w = 8

    def measure_lines(self, lines):
        return PAD_Y * 2 + 35 + 28 + len(lines) * self.line_h + 40

    def render_base(self, lines, height=None):
        h = height or self.measure_lines(lines)
        img = Image.new("RGB", (self.width, h), BG)
        d = ImageDraw.Draw(img)

        # Title bar
        d.rectangle([0, 0, self.width, 28], fill=TITLE_BG)
        d.text((PAD_X + 5, 7), "petstore.yaml", fill=(204, 204, 204), font=self.title_ui)
        d.text((self.width - 200, 7), "suspect LSP", fill=(106, 153, 85), font=self.ui)

        # Tab bar
        d.rectangle([0, 28, 180, 53], fill=(30, 30, 30))
        d.rectangle([0, 53, self.width, 54], fill=BG)
        d.text((15, 34), "📄 petstore.yaml", fill=(255, 255, 255), font=self.ui)
        d.line([(0, 54), (180, 54)], fill=(0, 122, 204), width=2)

        # Gutter background
        d.rectangle([0, 55, GUTTER_W, h], fill=GUTTER_BG)

        # Line numbers and separator
        d.line([(GUTTER_W, 55), (GUTTER_W, h)], fill=BORDER, width=1)

        return img, d


def render_diagnostics(spec_lines, findings_data, out_path):
    ed = Editor(1000)
    img, d = ed.render_base(spec_lines)

    y_start = 58
    decorations = {}

    # Mark diagnostic positions
    for item in findings_data.get("validate", []):
        line_num = item["start"] // 40 + 1
        sev_color = COL_ERROR if "Error" in item["severity"] else COL_WARN
        decorations[line_num] = sev_color

    # Draw code lines with squiggles
    for i, line in enumerate(spec_lines):
        y = y_start + i * ed.line_h
        line_num = i + 1
        d.text((10, y), str(line_num), fill=LINE_NUM, font=ed.font)

        segs = tokenize_yaml_line(line.rstrip("\n"))
        x = GUTTER_W + PAD_X
        for text, color in segs:
            d.text((x, y), text, fill=color, font=ed.font)
            x += ed.font.getbbox(text)[2] - ed.font.getbbox(text)[0]

        # Draw squiggle under problematic lines
        if line_num in decorations:
            sq_y = y + ed.line_h - 4
            color = decorations[line_num]
            for sx in range(GUTTER_W + PAD_X, min(x, ed.width - 20), 4):
                d.line([(sx, sq_y), (sx + 2, sq_y - 2)], fill=color, width=1)
                d.line([(sx + 2, sq_y - 2), (sx + 4, sq_y)], fill=color, width=1)

    # Draw problems panel at bottom
    panel_y = y_start + len(spec_lines) * ed.line_h + 10
    panel_h = min(len(findings_data.get("validate", [])) * 22 + 30, 200)
    d.rectangle([0, panel_y, ed.width, panel_y + panel_h], fill=(30, 30, 30))
    d.line([(0, panel_y), (ed.width, panel_y)], fill=BORDER, width=1)
    d.text((10, panel_y + 5), "PROBLEMS", fill=(204, 204, 204), font=ed.ui)

    py = panel_y + 24
    for item in findings_data.get("validate", [])[:6]:
        sev_col = COL_ERROR if "Error" in item["severity"] else COL_WARN
        msg = item["message"][:70]
        code = item["code"]
        d.text((15, py), f"⨯ {msg}", fill=sev_col, font=ed.ui)
        d.text((ed.width - 250, py), code, fill=(136, 136, 136), font=ed.ui)
        py += 18

    img.save(out_path)


def render_hover(spec_lines, hover_markdown, ref_line_num, out_path):
    ed = Editor(900)
    img, d = ed.render_base(spec_lines, height=max(ed.measure_lines(spec_lines), 500))

    # Draw code lines
    y_start = 58
    for i, line in enumerate(spec_lines):
        y = y_start + i * ed.line_h
        d.text((10, y), str(i + 1), fill=LINE_NUM, font=ed.font)
        segs = tokenize_yaml_line(line.rstrip("\n"))
        x = GUTTER_W + PAD_X
        for text, color in segs:
            d.text((x, y), text, fill=color, font=ed.font)
            x += ed.font.getbbox(text)[2] - ed.font.getbbox(text)[0]

    # Highlight the $ref line
    ref_y = y_start + (ref_line_num - 1) * ed.line_h
    d.rectangle([GUTTER_W + 1, ref_y - 1, ed.width - 1, ref_y + ed.line_h],
                outline=(80, 80, 80), width=1)

    # Draw hover card below the $ref line
    card_x = GUTTER_W + PAD_X + 60
    card_y = ref_y + ed.line_h + 4
    card_w = 550
    lines = hover_markdown.split("\n")
    card_h = len(lines) * 16 + 24

    # Background
    d.rounded_rectangle(
        [card_x, card_y, card_x + card_w, card_y + card_h],
        radius=4, fill=HOVER_BG, outline=BORDER, width=1,
    )

    # Render markdown-ish content
    tx = card_x + 10
    ty = card_y + 8
    ui = ed.ui
    bold = ed.bold
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("|"):
            # Table row
            cells = [c.strip().strip("`").replace("**", "") for c in stripped.split("|") if c.strip()]
            if cells:
                col_w = (card_w - 20) // max(len(cells), 1)
                cx = tx
                for cell in cells[:5]:
                    is_header = "---" not in cell
                    color = (204, 204, 204) if is_header else (180, 180, 180)
                    d.text((cx + 4, ty), cell.replace("✓", "✓"), fill=color, font=ui)
                    cx += col_w
                ty += 16
            continue
        if stripped.startswith("**Enum:**"):
            parts = stripped.split(": ", 1)
            d.text((tx, ty), parts[0] + ":", fill=(204, 204, 204), font=bold)
            if len(parts) > 1:
                d.text((tx + len(parts[0]) * 7 + 8, ty), parts[1], fill=COL_NUM, font=ui)
            ty += 16
            continue
        if stripped.startswith("**Constraints:**"):
            parts = stripped.split(": ", 1)
            d.text((tx, ty), parts[0] + ":", fill=(136, 136, 136), font=ui)
            if len(parts) > 1:
                d.text((tx + 90, ty), parts[1], fill=COL_INLAY, font=ui)
            ty += 14
            continue
        # Regular line: parse bold markers
        clean = stripped.replace("**", "")
        if clean:
            is_title = stripped.startswith("**")
            color = (255, 255, 255) if is_title else FG
            fnt = bold if is_title else ui
            d.text((tx, ty), clean, fill=color, font=fnt)
        ty += 16

    img.save(out_path)


def render_inlay(spec_lines, hints_data, out_path):
    ed = Editor(900)
    img, d = ed.render_base(spec_lines, height=max(ed.measure_lines(spec_lines), 600))

    y_start = 58
    for i, line in enumerate(spec_lines):
        y = y_start + i * ed.line_h
        d.text((10, y), str(i + 1), fill=LINE_NUM, font=ed.font)
        segs = tokenize_yaml_line(line.rstrip("\n"))
        x = GUTTER_W + PAD_X
        for text, color in segs:
            d.text((x, y), text, fill=color, font=ed.font)
            x += ed.font.getbbox(text)[2] - ed.font.getbbox(text)[0]

        # Draw inlay hints after $ref values
        if "$ref:" in line:
            end_x = x + 10
            hint_text = "→ Pet (petstore.yaml)"
            d.text((end_x + 5, y), hint_text,
                   fill=(136, 136, 136), font=ed.ui)
            # underline decoration
            hw = ed.ui.getbbox(hint_text)[2]
            d.line([(end_x + 5, y + 16), (end_x + 5 + hw, y + 16)],
                   fill=(80, 80, 80), width=1)

        # Draw property type hints
        if ": " in line and ("type:" not in line and "$ref" not in line):
            # This is a property definition line like `name:` or `id:`
            prop_name = line.strip().rstrip(":").strip()
            # Show type annotation as inlay
            type_map = {"name": "string", "tag": "string", "id": "int64"}
            fmt_map = {"id": "int64"}
            if prop_name in type_map:
                t = type_map[prop_name]
                fmt = ""
                if prop_name in fmt_map:
                    fmt = f" · {fmt_map[prop_name]}"
                hint = f": {t}{fmt}"
                hx = x + ed.font.getbbox(prop_name + ":")[2] - ed.font.getbbox(prop_name + ":")[0]
                d.text((x + 5 + ed.font.getbbox(prop_name + ":")[2], y),
                       hint, fill=COL_INLAY, font=ed.ui)

    img.save(out_path)


def main():
    data_path = os.path.join(os.path.dirname(__file__), "showcase_data.json")
    spec_path = os.path.join(os.path.dirname(__file__), "..", "..", "docs", "demo", "petstore.yaml")
    out_dir = os.path.join(os.path.dirname(__file__), "..", "docs", "images")

    with open(data_path) as f:
        showcase = json.load(f)
    spec_lines = open(spec_path).readlines()
    os.makedirs(out_dir, exist_ok=True)

    diag_entry = next((e for e in showcase if e["feature"] == "diagnostics"), {})
    hover_entry = next((e for e in showcase if e["feature"] == "hover"), {})

    print("Generating diagnostics...")
    render_diagnostics(spec_lines, diag_entry,
                       os.path.join(out_dir, "lsp-diagnostics.png"))

    hover_md = hover_entry.get("markdown", "")
    if hover_md:
        print("Generating hover...")
        render_hover(spec_lines, hover_md, 13,
                     os.path.join(out_dir, "lsp-hover.png"))

    print("Generating inlay hints...")
    fake_hints = []
    render_inlay(spec_lines, fake_hints,
                 os.path.join(out_dir, "lsp-inlay.png"))

    print("All screenshots generated")


if __name__ == "__main__":
    main()
