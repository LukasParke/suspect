//! Quoted scalar decoding shared by the CST and fast readers.

use std::borrow::Cow;

use crate::{Format, ScalarStyle};

/// Decodes a complete quoted JSON or YAML scalar, including its outer quotes.
///
/// Invalid UTF-8, malformed escapes, unpaired surrogates and invalid Unicode
/// scalar values return `None`; callers must not silently repair them. YAML
/// single quotes, extended double-quote escapes and flow-line folding follow
/// YAML 1.2. JSON accepts only its own escape forms and rejects raw controls.
#[must_use]
pub fn decode_quoted_scalar(
    text: &[u8],
    style: ScalarStyle,
    format: Format,
) -> Option<Cow<'_, [u8]>> {
    std::str::from_utf8(text).ok()?;
    let quote = match style {
        ScalarStyle::DoubleQuoted => b'"',
        ScalarStyle::SingleQuoted if format == Format::Yaml => b'\'',
        _ => return None,
    };
    if text.len() < 2 || text.first() != Some(&quote) || text.last() != Some(&quote) {
        return None;
    }
    let inner = &text[1..text.len() - 1];
    if !inner.iter().any(|&b| {
        b == quote || (quote == b'"' && b == b'\\') || b < 0x20 || b == b'\r' || b == b'\n'
    }) {
        return Some(Cow::Borrowed(inner));
    }
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let byte = inner[i];
        if byte == quote {
            if quote != b'\'' || inner.get(i + 1) != Some(&quote) {
                return None;
            }
            out.push(quote);
            i += 2;
        } else if quote == b'"' && byte == b'\\' {
            i += 1;
            let escaped = *inner.get(i)?;
            i += 1;
            if format == Format::Yaml && matches!(escaped, b'\n' | b'\r') {
                // Escaped line breaks contribute no space. Subsequent empty
                // lines still contribute their line breaks.
                skip_line_break(inner, &mut i, escaped);
                skip_indent(inner, &mut i);
                while inner.get(i).is_some_and(|b| matches!(b, b'\n' | b'\r')) {
                    let line_break = inner[i];
                    i += 1;
                    skip_line_break(inner, &mut i, line_break);
                    skip_indent(inner, &mut i);
                    out.push(b'\n');
                }
                continue;
            }
            let value = match escaped {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'0' if format == Format::Yaml => '\0',
                b'a' if format == Format::Yaml => '\u{7}',
                b'v' if format == Format::Yaml => '\u{b}',
                b'e' if format == Format::Yaml => '\u{1b}',
                b' ' if format == Format::Yaml => ' ',
                b'\t' if format == Format::Yaml => '\t',
                b'N' if format == Format::Yaml => '\u{85}',
                b'_' if format == Format::Yaml => '\u{a0}',
                b'L' if format == Format::Yaml => '\u{2028}',
                b'P' if format == Format::Yaml => '\u{2029}',
                b'u' | b'U' | b'x' => {
                    let width = match escaped {
                        b'u' => 4,
                        b'U' if format == Format::Yaml => 8,
                        b'x' if format == Format::Yaml => 2,
                        _ => return None,
                    };
                    let mut codepoint = hex_digits(inner, &mut i, width)?;
                    if (0xd800..=0xdbff).contains(&codepoint) && escaped == b'u' {
                        if inner.get(i..i + 2)? != b"\\u" {
                            return None;
                        }
                        i += 2;
                        let low = hex_digits(inner, &mut i, 4)?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return None;
                        }
                        codepoint = 0x10000 + ((codepoint - 0xd800) << 10) + (low - 0xdc00);
                    }
                    char::from_u32(codepoint)?
                }
                _ => return None,
            };
            let mut encoded = [0u8; 4];
            out.extend_from_slice(value.encode_utf8(&mut encoded).as_bytes());
        } else if format == Format::Yaml && matches!(byte, b'\n' | b'\r') {
            while out.last().is_some_and(|b| matches!(b, b' ' | b'\t')) {
                out.pop();
            }
            let mut breaks = 0;
            while inner.get(i).is_some_and(|b| matches!(b, b'\n' | b'\r')) {
                let line_break = inner[i];
                i += 1;
                skip_line_break(inner, &mut i, line_break);
                skip_indent(inner, &mut i);
                breaks += 1;
            }
            if breaks == 1 {
                out.push(b' ');
            } else {
                out.extend(std::iter::repeat_n(b'\n', breaks - 1));
            }
        } else {
            if byte < 0x20 && (format == Format::Json || byte != b'\t') {
                return None;
            }
            out.push(byte);
            i += 1;
        }
    }
    Some(Cow::Owned(out))
}

fn hex_digits(bytes: &[u8], offset: &mut usize, width: usize) -> Option<u32> {
    let digits = bytes.get(*offset..*offset + width)?;
    let mut value = 0;
    for &digit in digits {
        value = value * 16 + char::from(digit).to_digit(16)?;
    }
    *offset += width;
    Some(value)
}

fn skip_line_break(bytes: &[u8], offset: &mut usize, first: u8) {
    if first == b'\r' && bytes.get(*offset) == Some(&b'\n') {
        *offset += 1;
    }
}

fn skip_indent(bytes: &[u8], offset: &mut usize) {
    while bytes
        .get(*offset)
        .is_some_and(|b| matches!(b, b' ' | b'\t'))
    {
        *offset += 1;
    }
}
