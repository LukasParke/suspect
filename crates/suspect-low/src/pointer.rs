use std::fmt;

/// An RFC 6901 JSON Pointer.
///
/// Tokens are stored in their *unescaped* form (`~0`/`~1` already decoded).
/// Evaluation against documents goes through [`crate::NodeRef::pointer`],
/// which resolves YAML aliases transparently.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Pointer {
    tokens: Vec<Box<str>>,
}

/// Error produced when a string cannot be parsed as a JSON Pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointerError {
    /// The string that failed to parse, verbatim.
    pub input: String,
}

impl fmt::Display for PointerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON pointer: {:?}", self.input)
    }
}

impl std::error::Error for PointerError {}

impl Pointer {
    /// The root pointer (empty token list).
    #[must_use]
    pub fn root() -> Self {
        Self { tokens: Vec::new() }
    }

    /// Builds a pointer from already-unescaped tokens.
    #[must_use]
    pub fn from_tokens(tokens: Vec<Box<str>>) -> Self {
        Self { tokens }
    }

    /// Parses an RFC 6901 pointer string. Accepts both `#/a/b` (URI fragment
    /// form) and `/a/b`. The empty string and `#` are the root pointer.
    ///
    /// # Errors
    /// Returns a `PointerError` for non-pointer fragments (e.g. plain-name
    /// fragments like `#someAnchor`) — those are resolved by other means.
    pub fn parse(s: &str) -> Result<Self, PointerError> {
        let body = s.strip_prefix('#').unwrap_or(s);
        if body.is_empty() {
            return Ok(Self::root());
        }
        if !body.starts_with('/') {
            return Err(PointerError {
                input: s.to_owned(),
            });
        }
        let mut tokens = Vec::new();
        for raw in body.split('/').skip(1) {
            // unescape ~1 before ~0 (RFC 6901 §4); any other ~ is invalid
            let bytes = raw.as_bytes();
            let mut out = String::with_capacity(raw.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'~' {
                    match bytes.get(i + 1) {
                        Some(b'0') => {
                            out.push('~');
                            i += 2;
                        }
                        Some(b'1') => {
                            out.push('/');
                            i += 2;
                        }
                        _ => {
                            return Err(PointerError {
                                input: s.to_owned(),
                            });
                        }
                    }
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // multi-byte UTF-8 chars were pushed byte-wise above
            tokens.push(fix_utf8(out).into());
        }
        Ok(Self { tokens })
    }

    #[must_use]
    /// The unescaped tokens in order; `""` parses to an empty slice
    /// (the root pointer).
    pub fn tokens(&self) -> &[Box<str>] {
        &self.tokens
    }

    #[must_use]
    /// True when this pointer addresses the document root (no tokens).
    pub fn is_root(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Pointer to the parent, or `None` at the root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let mut tokens = self.tokens.clone();
        tokens.pop()?;
        Some(Self { tokens })
    }

    /// Appends one token (unescaped form).
    #[must_use]
    pub fn push(&self, token: &str) -> Self {
        let mut tokens = self.tokens.clone();
        tokens.push(token.into());
        Self { tokens }
    }

    /// Concatenates two pointers.
    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        let mut tokens = self.tokens.clone();
        tokens.extend(other.tokens.iter().cloned());
        Self { tokens }
    }

    /// Serializes to `/a/b` form with RFC 6901 escaping.
    #[must_use]
    pub fn to_path(&self) -> String {
        let mut out = String::new();
        for t in &self.tokens {
            out.push('/');
            for c in t.chars() {
                match c {
                    '~' => out.push_str("~0"),
                    '/' => out.push_str("~1"),
                    _ => out.push(c),
                }
            }
        }
        out
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_path())
    }
}

/// Percent-decodes a URI fragment body (`%XX` sequences); invalid escapes
/// pass through verbatim.
#[must_use]
pub fn percent_decode_fragment(frag: &str) -> Vec<u8> {
    let bytes = frag.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Reassembles UTF-8 from byte-wise unescaping.
fn fix_utf8(bytes: String) -> String {
    // out was built by pushing u8 as char — reconstruct properly
    let bytes: Vec<u8> = bytes.chars().map(|c| u32::from(c) as u8).collect();
    String::from_utf8_lossy(&bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(p: &Pointer) -> Vec<&str> {
        p.tokens().iter().map(|t| t.as_ref()).collect()
    }

    #[test]
    fn parse_forms() {
        assert!(Pointer::parse("").unwrap().is_root());
        assert!(Pointer::parse("#").unwrap().is_root());
        let p = Pointer::parse("#/components/schemas/Pet").unwrap();
        assert_eq!(toks(&p), ["components", "schemas", "Pet"]);
        let p2 = Pointer::parse("/paths/~1pets/get").unwrap();
        assert_eq!(toks(&p2), ["paths", "/pets", "get"]);
    }

    #[test]
    fn escapes_round_trip() {
        let p = Pointer::parse("/a~1b/c~0d").unwrap();
        assert_eq!(toks(&p), ["a/b", "c~d"]);
        assert_eq!(p.to_path(), "/a~1b/c~0d");
    }

    #[test]
    fn parent_and_push() {
        let p = Pointer::parse("#/a/b").unwrap();
        assert_eq!(p.parent().unwrap().to_path(), "/a");
        let root = p.parent().unwrap().parent().unwrap();
        assert!(root.is_root());
        assert!(root.parent().is_none());
        assert_eq!(p.push("c").to_path(), "/a/b/c");
        assert_eq!(Pointer::root().join(&p).to_path(), "/a/b");
    }

    #[test]
    fn invalid_escape_rejected() {
        assert!(Pointer::parse("/a~9").is_err());
        assert!(Pointer::parse("/~").is_err());
        // ~1 + literal 0 is legal: unescapes to "/0"
        assert_eq!(Pointer::parse("/~10").unwrap().tokens()[0].as_ref(), "/0");
        assert_eq!(
            Pointer::parse("/a~1b~0c").unwrap().tokens()[0].as_ref(),
            "a/b~c"
        );
    }

    #[test]
    fn non_pointer_fragment_rejected() {
        assert!(Pointer::parse("#someAnchor").is_err());
    }

    #[test]
    fn percent_decode() {
        assert_eq!(percent_decode_fragment("%7Bx%7D"), b"{x}".to_vec());
        assert_eq!(percent_decode_fragment("plain"), b"plain".to_vec());
        assert_eq!(percent_decode_fragment("%zz"), b"%zz".to_vec());
    }
}
