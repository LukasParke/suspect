//! Suspect Cassette: recorded HTTP traffic as header-plus-entries JSONL.
//!
//! Line 1 is a [`CassetteHeader`]; every following line is one
//! [`CassetteEntry`]. Bodies are stored either as UTF-8 or base64 with a
//! SHA-256 hash for integrity and fast matching. The format is append-only
//! and streamable; readers materialize all entries (an index is built by
//! consumers, not here).

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::sha256_hex;

/// Format identifier written in every cassette header.
pub const CASSETTE_FORMAT: &str = "suspect-cassette";

/// Current cassette format version.
pub const CASSETTE_VERSION: u32 = 1;

/// First line of a cassette file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteHeader {
    /// Always [`CASSETTE_FORMAT`] for files written by this version.
    pub format: String,
    /// Always [`CASSETTE_VERSION`] for files written by this version.
    pub version: u32,
    /// Unix epoch milliseconds of recording start.
    pub recorded_at_ms: u64,
    /// Human-readable origin (`spec.yaml`, `proxy https://api.example.com`).
    pub source: String,
}

/// How a body's `content` field is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyEncoding {
    /// Content is valid UTF-8 text.
    Utf8,
    /// Content is standard base64 of raw bytes.
    Base64,
}

/// A request or response body with integrity hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    /// Encoding of `content`.
    pub encoding: BodyEncoding,
    /// Encoded body content.
    pub content: String,
    /// SHA-256 hex digest of the raw bytes.
    pub sha256: String,
}

impl Body {
    /// Builds a body from raw bytes, choosing UTF-8 when valid.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self {
                encoding: BodyEncoding::Utf8,
                content: text.to_owned(),
                sha256: sha256_hex(bytes),
            },
            Err(_) => Self {
                encoding: BodyEncoding::Base64,
                content: base64_encode(bytes),
                sha256: sha256_hex(bytes),
            },
        }
    }

    /// Raw bytes after decoding.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        match self.encoding {
            BodyEncoding::Utf8 => self.content.clone().into_bytes(),
            BodyEncoding::Base64 => base64_decode(&self.content),
        }
    }

    /// The body as UTF-8 text when it was stored as UTF-8.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self.encoding {
            BodyEncoding::Utf8 => Some(&self.content),
            BodyEncoding::Base64 => None,
        }
    }
}

/// One recorded exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CassetteEntry {
    /// Sequence number within the cassette.
    pub id: u64,
    /// HTTP method.
    pub method: String,
    /// Full URL as requested (scheme/host included).
    pub url: String,
    /// Response status code.
    pub status: u16,
    /// Request headers (already redacted at record time).
    pub request_headers: Vec<(String, String)>,
    /// Request body.
    pub request_body: Body,
    /// Response headers (already redacted at record time).
    pub response_headers: Vec<(String, String)>,
    /// Response body.
    pub response_body: Body,
    /// Exchange duration in milliseconds.
    pub duration_ms: f64,
}

/// Writes a complete cassette: header line, then one line per entry.
///
/// # Errors
/// Propagates I/O errors from the writer.
pub fn write_cassette<W: Write>(
    w: &mut W,
    header: &CassetteHeader,
    entries: &[CassetteEntry],
) -> std::io::Result<()> {
    let header_line = serde_json::to_string(header)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(w, "{header_line}")?;
    for entry in entries {
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(w, "{line}")?;
    }
    Ok(())
}

/// Reads a complete cassette from any reader.
///
/// # Errors
/// `InvalidData` on malformed JSON, wrong format id, unsupported version,
/// or entry ids that are not strictly increasing; propagates I/O errors.
pub fn read_cassette<R: std::io::Read>(
    r: R,
) -> std::io::Result<(CassetteHeader, Vec<CassetteEntry>)> {
    let mut reader = std::io::BufReader::new(r);
    let mut header_line = String::new();
    let n = reader.read_line(&mut header_line)?;
    if n == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "empty cassette",
        ));
    }
    let header: CassetteHeader = {
        let parsed: Result<CassetteHeader, _> = serde_json::from_str(header_line.trim());
        parsed.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
    };
    if header.format != CASSETTE_FORMAT {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("not a {CASSETTE_FORMAT} file: {}", header.format),
        ));
    }
    if header.version > CASSETTE_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "cassette version {} newer than supported {}",
                header.version, CASSETTE_VERSION
            ),
        ));
    }
    let mut entries = Vec::new();
    let mut expected_id = 0u64;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: CassetteEntry = serde_json::from_str(line.trim())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        expected_id += 1;
        if entry.id != expected_id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "entry id {} out of sequence (expected {expected_id})",
                    entry.id
                ),
            ));
        }
        entries.push(entry);
    }
    Ok((header, entries))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(text: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap_or_default()
}
