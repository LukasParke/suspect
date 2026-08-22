use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Files at or above this size are memory-mapped instead of read.
pub const MMAP_THRESHOLD: u64 = 256 * 1024;

/// Source text encoding as detected from the BOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

enum Data {
    Owned(Box<[u8]>),
    Mapped(memmap2::Mmap),
}

/// A loaded document buffer, guaranteed UTF-8.
///
/// BOMs are stripped and UTF-16 inputs are transcoded once at load; every
/// downstream byte offset refers to the decoded buffer. Invalid UTF-8 is kept
/// as-is (tree-sitter reports it); `str` accessors must tolerate that.
pub struct Source {
    data: Data,
    encoding: Encoding,
}

impl Source {
    /// Loads from the filesystem: mmap for large files, pooled read otherwise.
    ///
    /// # Errors
    /// Filesystem errors.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len >= MMAP_THRESHOLD {
            // SAFETY: mapping a file opened read-only; no writes occur through
            // this mapping and the buffer is immutable for the process lifetime.
            let map = unsafe { memmap2::Mmap::map(&file)? };
            return Ok(Self::finish(Data::Mapped(map), Encoding::Utf8));
        }
        let mut buf = Vec::with_capacity(len as usize);
        (&file).read_to_end(&mut buf)?;
        Ok(Self::from_vec(buf))
    }

    /// Adopts an in-memory buffer.
    #[must_use]
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self::finish(Data::Owned(bytes.into()), Encoding::Utf8)
    }

    fn finish(data: Data, detected: Encoding) -> Self {
        let bytes: &[u8] = match &data {
            Data::Owned(b) => b,
            Data::Mapped(m) => m,
        };
        match detect_bom(bytes) {
            Some((Encoding::Utf8, skip)) => {
                let owned = bytes[skip..].to_vec().into_boxed_slice();
                Source {
                    data: Data::Owned(owned),
                    encoding: Encoding::Utf8,
                }
            }
            Some((enc @ (Encoding::Utf16Le | Encoding::Utf16Be), skip)) => {
                let transcoded = transcode_utf16(&bytes[skip..], enc);
                Source {
                    data: Data::Owned(transcoded.into()),
                    encoding: enc,
                }
            }
            None => Source {
                data,
                encoding: detected,
            },
        }
    }

    /// The decoded UTF-8 contents.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        match &self.data {
            Data::Owned(b) => b,
            Data::Mapped(m) => m,
        }
    }

    /// Interprets the whole buffer as `str` if it is valid UTF-8.
    #[must_use]
    pub fn str(&self) -> Option<&str> {
        std::str::from_utf8(self.bytes()).ok()
    }

    #[must_use]
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes().is_empty()
    }
}

fn detect_bom(bytes: &[u8]) -> Option<(Encoding, usize)> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((Encoding::Utf8, 3))
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        Some((Encoding::Utf16Le, 2))
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some((Encoding::Utf16Be, 2))
    } else {
        None
    }
}

fn transcode_utf16(bytes: &[u8], enc: Encoding) -> Vec<u8> {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    #[allow(clippy::manual_slice_size_calculation)]
    let mut chunks = bytes.chunks_exact(2);
    for chunk in &mut chunks {
        let u = match enc {
            Encoding::Utf16Le => u16::from_le_bytes([chunk[0], chunk[1]]),
            _ => u16::from_be_bytes([chunk[0], chunk[1]]),
        };
        units.push(u);
    }
    let mut out = String::with_capacity(units.len());
    let mut iter = units.into_iter().peekable();
    while let Some(u) = iter.next() {
        if (0xD800..0xDC00).contains(&u) {
            match iter.peek() {
                Some(&lo) if (0xDC00..0xE000).contains(&lo) => {
                    iter.next();
                    let c = 0x1_0000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(lo) - 0xDC00);
                    out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                }
                _ => out.push('\u{FFFD}'),
            }
        } else {
            out.push(char::from_u32(u32::from(u)).unwrap_or('\u{FFFD}'));
        }
    }
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_bom_stripped() {
        let src = Source::from_vec(b"\xEF\xBB\xBFhello".to_vec());
        assert_eq!(src.bytes(), b"hello");
        assert_eq!(src.encoding(), Encoding::Utf8);
    }

    #[test]
    fn utf16le_transcoded() {
        let text = "héllo";
        let mut buf = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            buf.extend_from_slice(&unit.to_le_bytes());
        }
        let src = Source::from_vec(buf);
        assert_eq!(src.encoding(), Encoding::Utf16Le);
        assert_eq!(src.str(), Some(text));
    }

    #[test]
    fn utf16be_surrogate_pair() {
        let mut buf = vec![0xFE, 0xFF];
        for unit in "a\u{1F600}b".encode_utf16() {
            buf.extend_from_slice(&unit.to_be_bytes());
        }
        let src = Source::from_vec(buf);
        assert_eq!(src.encoding(), Encoding::Utf16Be);
        assert_eq!(src.str(), Some("a\u{1F600}b"));
    }

    #[test]
    fn lone_surrogate_becomes_replacement_and_keeps_next_unit() {
        let mut buf = vec![0xFF, 0xFE];
        buf.extend_from_slice(&0xD800u16.to_le_bytes());
        buf.extend_from_slice(b"a\0");
        let src = Source::from_vec(buf);
        assert_eq!(src.str(), Some("\u{FFFD}a"));
    }

    #[test]
    fn small_file_loads_via_read() {
        let path = std::env::temp_dir().join("suspect-src-small.yaml");
        std::fs::write(&path, b"a: 1").unwrap();
        let src = Source::from_path(&path).unwrap();
        assert_eq!(src.bytes(), b"a: 1");
    }

    #[test]
    fn empty_file_is_empty_source() {
        let path = std::env::temp_dir().join("suspect-src-empty.yaml");
        std::fs::write(&path, b"").unwrap();
        let src = Source::from_path(&path).unwrap();
        assert!(src.is_empty());
    }
}
