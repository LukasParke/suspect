use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::Utf8Error;
use std::sync::Arc;

use url::Url;

/// Errors produced when parsing or joining URIs.
#[derive(Debug, thiserror::Error)]
pub enum UriError {
    #[error("invalid URI: {0}")]
    Invalid(String),
    #[error("relative URI without a base")]
    NoBase,
    #[error("path is not valid UTF-8")]
    NotUtf8,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// A canonical, fragment-free document identifier (`file://`, `http://`, ...).
///
/// Constructed through [`Uri::from_path`] or [`Uri::parse`]; equal documents
/// loaded via different spellings produce equal `Uri`s, which is what makes it
/// usable as workspace identity and memo-cache key.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uri(Arc<str>);

impl Uri {
    /// Builds a canonical URI from a filesystem path (absolute or relative to
    /// the current directory). Lexically resolves `.` and `..`.
    pub fn from_path(path: &Path) -> Result<Self, UriError> {
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let normalized = normalize_path(&abs);
        let url = Url::from_file_path(&normalized)
            .map_err(|_| UriError::Invalid(normalized.display().to_string()))?;
        Ok(Self(url.as_str().into()))
    }

    /// Parses an absolute URI (`file://`, `https://`, ...) or an absolute
    /// filesystem path. Fragments are rejected: a `Uri` identifies a document,
    /// not a position inside it.
    pub fn parse(s: &str) -> Result<Self, UriError> {
        if s.contains('#') {
            return Err(UriError::Invalid(format!(
                "fragment not allowed in document URI: {s}"
            )));
        }
        if let Ok(url) = Url::parse(s)
            && (url.has_host() || url.scheme() == "file")
        {
            return Ok(Self(url.as_str().into()));
        }
        let p = Path::new(s);
        if p.is_absolute() {
            return Self::from_path(p);
        }
        Err(UriError::Invalid(s.to_owned()))
    }

    /// Joins an RFC 3986 reference (possibly relative) against this base,
    /// returning a fragment-free document URI.
    pub fn join(&self, reference: &str) -> Result<Self, UriError> {
        let reference = reference.trim();
        if reference.is_empty() {
            return Ok(self.clone());
        }
        if reference.contains('#') {
            let (without_frag, _) = split_fragment(reference);
            return self.join(without_frag);
        }
        let base = Url::parse(self.as_str()).map_err(|_| UriError::Invalid(self.to_string()))?;
        let joined = base
            .join(reference)
            .map_err(|_| UriError::Invalid(reference.to_owned()))?;
        Ok(Self(joined.as_str().into()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Filesystem path for `file://` URIs.
    #[must_use]
    pub fn as_path(&self) -> Option<PathBuf> {
        let url = Url::parse(self.as_str()).ok()?;
        if url.scheme() != "file" {
            return None;
        }
        url.to_file_path().ok()
    }

    /// True for schemes that require network fetches.
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.scheme(), "http" | "https")
    }

    /// The URI scheme, lowercased (`"file"`, `"https"`, ...).
    #[must_use]
    pub fn scheme(&self) -> &str {
        match self.0.find(':') {
            Some(i) => &self.0[..i],
            None => "",
        }
    }

    /// Splits a `$ref`-style value into its document part and fragment part.
    /// `(None, fragment)` means same-document. Returns the fragment without
    /// the leading `#`; an absent fragment yields `""`.
    pub fn split_ref(value: &str) -> (Option<&str>, &str) {
        let (doc, frag) = split_fragment(value);
        (Some(doc).filter(|s| !s.is_empty()), frag)
    }
}

impl From<String> for Uri {
    fn from(s: String) -> Self {
        // Caller-provided URIs are trusted to be absolute; parse validates.
        match Uri::parse(&s) {
            Ok(u) => u,
            Err(_) => Uri(s.into()),
        }
    }
}

impl From<&str> for Uri {
    fn from(s: &str) -> Self {
        Uri::from(s.to_owned())
    }
}

fn split_fragment(s: &str) -> (&str, &str) {
    match s.find('#') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uri({})", self.0)
    }
}

impl TryFrom<&[u8]> for Uri {
    type Error = Utf8Error;
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Uri(std::str::from_utf8(value)?.into()))
    }
}

/// Lexically resolves `.` and `..` without touching the filesystem.
#[must_use]
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_normalizes_dot_segments() {
        let cwd = std::env::current_dir().unwrap();
        let u = Uri::from_path(Path::new("./a/b/../c.yaml")).unwrap();
        let expected = format!("file://{}/a/c.yaml", normalize_path(&cwd).display());
        assert_eq!(u.as_str(), expected);
    }

    #[test]
    fn parse_accepts_uris_and_absolute_paths() {
        assert_eq!(
            Uri::parse("file:///x/y.json").unwrap().as_str(),
            "file:///x/y.json"
        );
        assert!(Uri::parse("#/components").is_err());
        assert!(Uri::parse("relative.yaml").is_err());
    }

    #[test]
    fn join_resolves_relative_references() {
        let base = Uri::parse("file:///api/main.yaml").unwrap();
        assert_eq!(
            base.join("schemas/pet.yaml").unwrap().as_str(),
            "file:///api/schemas/pet.yaml"
        );
        assert_eq!(
            base.join("../common/headers.yaml").unwrap().as_str(),
            "file:///common/headers.yaml"
        );
        assert_eq!(base.join("").unwrap().as_str(), "file:///api/main.yaml");
    }

    #[test]
    fn join_strips_fragments() {
        let base = Uri::parse("file:///api/main.yaml").unwrap();
        assert_eq!(
            base.join("other.yaml#/components/schemas/Pet")
                .unwrap()
                .as_str(),
            "file:///api/other.yaml"
        );
    }

    #[test]
    fn split_ref_handles_all_shapes() {
        assert_eq!(Uri::split_ref("#/components/A"), (None, "/components/A"));
        assert_eq!(Uri::split_ref("b.yaml#/x"), (Some("b.yaml"), "/x"));
        assert_eq!(Uri::split_ref("b.yaml"), (Some("b.yaml"), ""));
    }

    #[test]
    fn as_path_round_trips() {
        let p = std::env::temp_dir().join("suspect-uri-test.yaml");
        let u = Uri::from_path(&p).unwrap();
        assert_eq!(u.as_path().unwrap(), normalize_path(&p));
    }
}
