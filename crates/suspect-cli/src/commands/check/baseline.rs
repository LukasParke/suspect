//! Findings baselines: record today's findings so CI fails only on new
//! ones (`suspect check --write-baseline` / `--baseline`, `check.baseline`).
//!
//! A finding's key is its canonical code, its document's path relative to
//! the baseline file, and its JSON pointer. There is no message or line in
//! the key: wording changes between releases and edits move lines, while a
//! pointer survives both. The file stores a count per key; a finding is
//! new when the run has more findings with its key than the file records.
//!
//! ```json
//! { "version": 1, "tool": "suspect",
//!   "findings": [{ "code": "oas-operation-missing-operationId",
//!                  "document": "docs/demo/petstore.yaml",
//!                  "pointer": "/paths/~1pets/get", "count": 1 }] }
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use suspect_diag::Finding;

/// The file format version this build reads and writes.
const VERSION: u32 = 1;

/// A finding's baseline identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    /// Document path relative to the baseline file, `/`-separated.
    pub document: String,
    /// RFC 6901 pointer; `""` for a document-level finding.
    pub pointer: String,
    /// Canonical code.
    pub code: String,
}

impl Key {
    /// The key of `finding`, with its document relative to `base` (the
    /// baseline file's directory).
    #[must_use]
    pub fn of(finding: &Finding, base: &Path) -> Self {
        let document = finding.document.as_path().map_or_else(
            || finding.document.to_string(),
            |path| relative(&path, base),
        );
        Self {
            document,
            pointer: finding.pointer.as_deref().unwrap_or("").to_owned(),
            code: finding.code.to_string(),
        }
    }

    /// SARIF `partialFingerprints["suspect/v1"]`: SHA-256 over code,
    /// document and pointer, NUL-separated.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(self.code.as_bytes());
        hash.update([0]);
        hash.update(self.document.as_bytes());
        hash.update([0]);
        hash.update(self.pointer.as_bytes());
        format!("{:x}", hash.finalize())
    }
}

/// `path` relative to `base`, `/`-separated, with `..` where `path` is not
/// below `base`; `path` itself when they share no root (another drive).
fn relative(path: &Path, base: &Path) -> String {
    use std::path::Component;
    let mut path_parts = path.components().peekable();
    let mut base_parts = base.components().peekable();
    if path_parts.peek() != base_parts.peek() {
        return path.to_string_lossy().replace('\\', "/");
    }
    while let (Some(a), Some(b)) = (path_parts.peek(), base_parts.peek()) {
        if a != b {
            break;
        }
        path_parts.next();
        base_parts.next();
    }
    let mut parts: Vec<String> = base_parts
        .filter(|c| matches!(c, Component::Normal(_)))
        .map(|_| "..".to_owned())
        .collect();
    parts.extend(path_parts.map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    tool: String,
    findings: Vec<Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    code: String,
    document: String,
    pointer: String,
    count: u32,
}

/// A loaded baseline file.
#[derive(Debug, Clone)]
pub struct Baseline {
    base: PathBuf,
    counts: HashMap<Key, u32>,
}

impl Baseline {
    /// Reads a baseline file.
    ///
    /// # Errors
    /// The file cannot be read, is not a version 1 baseline, or names a
    /// key twice (exit 2).
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read baseline {}", path.display()))?;
        let file: File = serde_json::from_str(&text)
            .with_context(|| format!("{} is not a suspect findings baseline", path.display()))?;
        anyhow::ensure!(
            file.version == VERSION,
            "{}: baseline version {} is not supported (expected {VERSION})",
            path.display(),
            file.version
        );
        let mut counts = HashMap::new();
        for entry in file.findings {
            let key = Key {
                document: entry.document,
                pointer: entry.pointer,
                code: entry.code,
            };
            anyhow::ensure!(
                counts.insert(key.clone(), entry.count).is_none(),
                "{}: `{}` at {}#{} is listed twice",
                path.display(),
                key.code,
                key.document,
                key.pointer
            );
        }
        Ok(Self {
            base: base_of(path),
            counts,
        })
    }

    /// The directory document paths are relative to.
    #[must_use]
    pub fn base(&self) -> PathBuf {
        self.base.clone()
    }

    /// For each key, in order, whether the baseline covers it: the first
    /// `count` findings with a key are covered, the rest are new.
    #[must_use]
    pub fn covers(&self, keys: &[Key]) -> Vec<bool> {
        let mut used: HashMap<&Key, u32> = HashMap::new();
        keys.iter()
            .map(|key| {
                let stored = self.counts.get(key).copied().unwrap_or(0);
                let seen = used.entry(key).or_default();
                *seen += 1;
                *seen <= stored
            })
            .collect()
    }
}

/// The directory a baseline at `path` is relative to.
fn base_of(path: &Path) -> PathBuf {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    dir.canonicalize().unwrap_or(dir)
}

/// Keys are relative to the working directory when no baseline is in use.
#[must_use]
pub fn default_base() -> PathBuf {
    std::env::current_dir()
        .and_then(|dir| dir.canonicalize())
        .unwrap_or_default()
}

/// Writes `findings` as a baseline at `path`, sorted by document, pointer
/// and code, ending with a newline. Returns how many findings it records.
///
/// # Errors
/// Filesystem IO.
pub fn write(path: &Path, findings: &[Finding]) -> anyhow::Result<usize> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let base = base_of(path);
    let mut counts: BTreeMap<Key, u32> = BTreeMap::new();
    for finding in findings {
        *counts.entry(Key::of(finding, &base)).or_default() += 1;
    }
    let file = File {
        version: VERSION,
        tool: "suspect".to_owned(),
        findings: counts
            .into_iter()
            .map(|(key, count)| Entry {
                code: key.code,
                document: key.document,
                pointer: key.pointer,
                count,
            })
            .collect(),
    };
    std::fs::write(path, serde_json::to_string_pretty(&file)? + "\n")
        .with_context(|| format!("cannot write baseline {}", path.display()))?;
    Ok(findings.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_diag::{CheckId, Severity};
    use suspect_source::Uri;

    fn finding(code: &'static str, pointer: &str) -> Finding {
        let mut f = Finding::new(
            CheckId::Validate,
            code,
            Severity::Error,
            Uri::from_path(Path::new("/w/specs/api.yaml")).unwrap(),
            "m",
        );
        f.pointer = Some(pointer.into());
        f
    }

    #[test]
    fn keys_are_relative_and_counts_cover_the_first_findings() {
        let base = Path::new("/w");
        let key = Key::of(&finding("a", "/paths"), base);
        assert_eq!(key.document, "specs/api.yaml");
        assert_eq!(key.pointer, "/paths");
        let baseline = Baseline {
            base: base.to_path_buf(),
            counts: HashMap::from([(key.clone(), 1)]),
        };
        let other = Key::of(&finding("b", "/paths"), base);
        assert_eq!(
            Key::of(&finding("a", ""), Path::new("/w/specs/.suspect")).document,
            "../api.yaml"
        );
        assert_eq!(
            Key::of(&finding("a", ""), Path::new("/x/y")).document,
            "../../w/specs/api.yaml"
        );
        assert_eq!(
            baseline.covers(&[key.clone(), other, key]),
            [true, false, false]
        );
    }

    #[test]
    fn files_round_trip_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/baseline.json");
        let findings = [finding("b", "/x"), finding("a", "/x"), finding("a", "/x")];
        write(&path, &findings).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with("}\n"));
        let a = text.find("\"code\": \"a\"").unwrap();
        assert!(a < text.find("\"code\": \"b\"").unwrap(), "{text}");
        assert!(text.contains("\"count\": 2"), "{text}");
        let read = Baseline::read(&path).unwrap();
        assert_eq!(read.counts.len(), 2);
        std::fs::write(
            &path,
            "{\"version\": 9, \"tool\": \"suspect\", \"findings\": []}",
        )
        .unwrap();
        assert!(Baseline::read(&path).is_err());
        std::fs::write(&path, "not json").unwrap();
        assert!(Baseline::read(&path).is_err());
    }
}
