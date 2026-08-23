//! Editor color swatches (`#RRGGBB`, `#RRGGBBAA`, `#RGB`, `#RGBA`)
//! inside string values.
//!
//! OpenAPI specs carry colors in examples, descriptions, and branding
//! metadata (`info.contact`, theme tokens); rendering them as editor
//! swatches makes those values visible without running the spec. The scan
//! is purely lexical over scalar leaves — no schema knowledge required —
//! and deliberately conservative: a `#` run must be entirely hex digits of
//! an exact legal length (3/4/6/8), so URLs like `/pets/{id}#L4` or
//! anchors never produce false swatches.

use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{Color, ColorInformation, ColorPresentation, TextEdit};

use crate::state::{OpenDoc, lsp_range};

/// Parses one hex color literal starting at `text`'s beginning.
///
/// Accepts `#RGB`, `#RGBA`, `#RRGGBB`, and `#RRGGBBAA`; anything shorter,
pub fn parse_hex_color(text: &str) -> Option<(usize, Color)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'#') {
        return None;
    }
    // Maximal run of hex digits decides both validity and length: a
    // trailing non-hex byte (`#00FF00 ok`) or end-of-string terminates
    // the literal, while runs of other sizes (`#frag`, `#12345`) never
    // form a color.
    let run = bytes[1..]
        .iter()
        .take_while(|b| b.is_ascii_hexdigit())
        .count();
    let hex = &text[1..1 + run];
    // CSS digit-doubling: `#RGB`/`#RGBA` expand each nibble (`A` → `AA`),
    // so a single hex digit contributes value*17, not its raw value.
    let doubled = |s: &str| -> Option<f32> {
        u8::from_str_radix(s, 16)
            .ok()
            .map(|v| f32::from(v * 17) / 255.0)
    };
    let channel =
        |s: &str| -> Option<f32> { u8::from_str_radix(s, 16).ok().map(|v| f32::from(v) / 255.0) };
    let (r, g, b, a): (f32, f32, f32, f32) = match run {
        3 => (
            doubled(&hex[..1])?,
            doubled(&hex[1..2])?,
            doubled(&hex[2..3])?,
            1.0,
        ),
        4 => (
            doubled(&hex[..1])?,
            doubled(&hex[1..2])?,
            doubled(&hex[2..3])?,
            doubled(&hex[3..4])?,
        ),
        6 => (
            channel(&hex[..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            1.0,
        ),
        8 => (
            channel(&hex[..2])?,
            channel(&hex[2..4])?,
            channel(&hex[4..6])?,
            channel(&hex[6..8])?,
        ),
        _ => return None,
    };
    Some((
        1 + run,
        Color {
            red: r,
            green: g,
            blue: b,
            alpha: a,
        },
    ))
}
/// Renders a color back into canonical hex form.
///
/// Channels are quantized to 8-bit precision; alpha below 1.0 emits an
/// eight-digit literal, full alpha the six-digit form.
#[must_use]
pub fn to_hex(color: &Color) -> String {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, b, a) = (
        byte(color.red),
        byte(color.green),
        byte(color.blue),
        byte(color.alpha),
    );
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

/// Finds every hex color literal in the document's scalar values.
#[must_use]
pub fn document_colors(doc: &OpenDoc) -> Vec<ColorInformation> {
    let inner = doc.low.inner();
    let bytes = inner.bytes();
    let li = inner.line_index();
    let mut out = Vec::new();
    for n in inner.root().descendants() {
        if n.kind() != SyntaxKind::Scalar {
            continue;
        }
        let Ok(text) = std::str::from_utf8(n.content().scalar_bytes()) else {
            continue;
        };
        let base = n.content().byte_range().start;
        let mut search = 0usize;
        while let Some(hash) = text[search..].find('#') {
            let start = search + hash;
            if let Some((len, color)) = parse_hex_color(&text[start..]) {
                out.push(ColorInformation {
                    range: lsp_range(bytes, li, (base + start)..(base + start + len)),
                    color,
                });
                search = start + len;
            } else {
                search = start + 1;
            }
        }
    }
    out
}

/// Builds the pick-a-swatch replacement presentation for one color.
///
/// A single canonical-hex edit anchored on the original literal's range;
/// clients show it when the user picks a color from the picker UI.
#[must_use]
pub fn color_presentations(
    color: &Color,
    range: tower_lsp::lsp_types::Range,
) -> Vec<ColorPresentation> {
    let hex = to_hex(color);
    vec![ColorPresentation {
        label: hex.clone(),
        text_edit: Some(TextEdit {
            range,
            new_text: hex,
        }),
        additional_text_edits: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::OpenDoc;
    use suspect_source::Uri;

    const URI: &str = "file:///mem/colors.yaml";

    fn open(text: &str) -> OpenDoc {
        OpenDoc::parse(Uri::parse(URI).unwrap(), text.to_owned())
    }

    #[test]
    fn parses_all_four_lengths() {
        let (_, c6) = parse_hex_color("#FF0000").unwrap();
        assert_eq!(c6.red, 1.0);
        assert_eq!(c6.green, 0.0);
        assert_eq!(c6.alpha, 1.0);
        let (_, c3) = parse_hex_color("#0AF").unwrap();
        assert!((c3.red - 0.0).abs() < 1e-6);
        assert!((c3.green - 170.0 / 255.0).abs() < 1e-6);
        let (_, c8) = parse_hex_color("#00000080").unwrap();
        assert!((c8.alpha - 128.0 / 255.0).abs() < 1e-6);
        assert!(parse_hex_color("#12345").is_none());
        assert!(parse_hex_color("#GGGGGG").is_none());
        assert!(parse_hex_color("FF0000").is_none());
    }

    #[test]
    fn finds_colors_and_skips_urls_and_anchors() {
        let doc = open(
            "openapi: 3.1.0\ninfo:\n  title: T\npaths:\n  /a#frag:\n    get:\n      responses:\n        '200':\n          description: \"theme #00FF00 ok\"\n",
        );
        let colors = document_colors(&doc);
        assert_eq!(colors.len(), 1, "{colors:?}");
        assert_eq!(colors[0].color.green, 1.0);
    }

    #[test]
    fn bare_hash_in_plain_scalar_is_a_comment_not_a_color() {
        // In YAML, `#` preceded by whitespace starts a comment; the tree
        // puts `#00FF00` in a Comment node and the scanner must agree.
        let doc = open("description: theme #00FF00\n");
        assert!(document_colors(&doc).is_empty());
    }

    #[test]
    fn json_scalars_get_swatches_too() {
        let doc = OpenDoc::parse(
            Uri::parse("file:///mem/colors.json").unwrap(),
            "{\"description\": \"accent #ff8800\"}".to_owned(),
        );
        let colors = document_colors(&doc);
        assert_eq!(colors.len(), 1, "{colors:?}");
    }

    #[test]
    fn presentations_round_trip_through_hex() {
        let doc = open("description: '#00FF00'\n");
        let found = document_colors(&doc);
        assert_eq!(found.len(), 1, "quoted literal gains a swatch");
        let (_, parsed) = parse_hex_color("#00FF00").unwrap();
        let range = tower_lsp::lsp_types::Range::default();
        let pres = color_presentations(&parsed, range);
        assert_eq!(pres.len(), 1);
        assert_eq!(pres[0].label, "#00FF00");
        assert_eq!(pres[0].text_edit.as_ref().unwrap().new_text, "#00FF00");
        // Alpha survives a round trip.
        let (_, translucent) = parse_hex_color("#00FF0080").unwrap();
        assert_eq!(to_hex(&translucent), "#00FF0080");
    }

    #[test]
    fn to_hex_quantizes_channels() {
        let c = Color {
            red: 0.5,
            green: 0.254,
            blue: 0.0,
            alpha: 1.0,
        };
        assert_eq!(to_hex(&c), "#804100");
    }
}
