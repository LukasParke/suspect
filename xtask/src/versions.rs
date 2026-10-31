//! `bump` and `check-versions`: one lockstep version for the workspace
//! crates, both lockfiles, the editor extensions and the rules SDK
//! (docs/proposals/P5-release.md, "Versioning").
//!
//! Edits are targeted byte-range replacements, so file formatting is kept.
//! The source of truth is `[workspace.package] version` in `Cargo.toml`.

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub const BUMP_USAGE: &str = "usage: cargo xtask bump <version>";
pub const CHECK_USAGE: &str = "usage: cargo xtask check-versions [--tag v<version>]";

/// One version string in one file.
#[derive(Debug)]
struct Site {
    label: String,
    span: Range<usize>,
}

struct VersionFile {
    rel: &'static str,
    text: String,
    sites: Vec<Site>,
}

impl VersionFile {
    fn value(&self, site: &Site) -> &str {
        &self.text[site.span.clone()]
    }
}

/// Files that carry the toolchain version, relative to the workspace root.
const FILES: &[(&str, Kind)] = &[
    ("Cargo.toml", Kind::RootManifest),
    ("Cargo.lock", Kind::CargoLock),
    ("editors/zed/Cargo.toml", Kind::TomlKey(Some("package"))),
    ("editors/zed/Cargo.lock", Kind::CargoLock),
    ("editors/zed/extension.toml", Kind::TomlKey(None)),
    ("editors/vscode/package.json", Kind::Json(&[&["version"]])),
    (
        "editors/vscode/package-lock.json",
        Kind::Json(&[&["version"], &["packages", "", "version"]]),
    ),
    ("rules-runtime/package.json", Kind::Json(&[&["version"]])),
];

#[derive(Clone, Copy)]
enum Kind {
    /// `[workspace.package] version` plus every internal
    /// `[workspace.dependencies]` entry's `version`.
    RootManifest,
    /// Every path (source-less) package in a lockfile.
    CargoLock,
    /// `version` in the given table (`None`: before the first header).
    TomlKey(Option<&'static str>),
    /// String values at these object-key paths.
    Json(&'static [&'static [&'static str]]),
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

pub fn run_bump(args: &[String]) -> Result<()> {
    let [version] = args else {
        bail!("{BUMP_USAGE}");
    };
    if matches!(version.as_str(), "-h" | "--help") {
        println!("{BUMP_USAGE}");
        return Ok(());
    }
    let root = workspace_root();
    let changed = bump(&root, version)?;
    for rel in &changed {
        println!("updated {rel}");
    }
    println!("all version lines are {version}");
    prepend_changelog(&root, version)
}

/// Prepends the commits since the last tag to `CHANGELOG.md` with
/// git-cliff (`cliff.toml`). Without git-cliff the bump still succeeds and
/// says how to add the section.
fn prepend_changelog(root: &Path, version: &str) -> Result<()> {
    let tag = format!("v{version}");
    let args = [
        "--unreleased",
        "--tag",
        tag.as_str(),
        "--prepend",
        "CHANGELOG.md",
    ];
    let shown = format!("git cliff {}", args.join(" "));
    match std::process::Command::new("git-cliff")
        .args(args)
        .current_dir(root)
        .status()
    {
        Ok(status) if status.success() => {
            println!("updated CHANGELOG.md ({shown})");
            Ok(())
        }
        Ok(status) => bail!("`{shown}` exited with {status}"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "git-cliff not found; CHANGELOG.md not updated. Install it \
                 (cargo install --locked git-cliff) and run `{shown}`"
            );
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("spawn `{shown}`")),
    }
}

pub fn run_check(args: &[String]) -> Result<()> {
    let mut tag = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tag" => tag = Some(it.next().context(CHECK_USAGE)?.as_str()),
            "-h" | "--help" => {
                println!("{CHECK_USAGE}");
                return Ok(());
            }
            other => bail!("check-versions: unknown argument {other:?}\n{CHECK_USAGE}"),
        }
    }
    let version = check_versions(&workspace_root(), tag)?;
    println!("all version lines are {version}");
    Ok(())
}

/// Sets every version line to `version`. Returns the files that changed.
pub fn bump(root: &Path, version: &str) -> Result<Vec<&'static str>> {
    validate_version(version)?;
    let mut changed = Vec::new();
    for file in load(root)? {
        let mut text = file.text.clone();
        let mut sites: Vec<&Site> = file.sites.iter().collect();
        sites.sort_by_key(|s| std::cmp::Reverse(s.span.start));
        for site in sites {
            text.replace_range(site.span.clone(), version);
        }
        if text != file.text {
            let path = root.join(file.rel);
            std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
            changed.push(file.rel);
        }
    }
    check_versions(root, None)?;
    Ok(changed)
}

/// Fails when any version line differs from `[workspace.package] version`
/// or, with `tag` (`v{V}`), from the tag. Returns the workspace version.
pub fn check_versions(root: &Path, tag: Option<&str>) -> Result<String> {
    let files = load(root)?;
    let root_manifest = &files[0];
    let reference = &root_manifest.sites[0];
    if !reference.label.starts_with("[workspace.package]") {
        bail!("Cargo.toml: no [workspace.package] version");
    }
    let expected = root_manifest.value(reference).to_owned();
    let mut errors = Vec::new();
    for file in &files {
        for site in &file.sites {
            let found = file.value(site);
            if found != expected {
                errors.push(format!(
                    "{}: {} is {found:?}, expected {expected:?}",
                    file.rel, site.label
                ));
            }
        }
    }
    errors.extend(member_manifest_errors(root)?);
    if let Some(tag) = tag {
        let tagged = tag.strip_prefix('v').unwrap_or(tag);
        if tagged != expected {
            errors.push(format!(
                "tag {tag:?} does not match workspace version {expected:?}"
            ));
        }
    }
    if !errors.is_empty() {
        bail!(
            "{} version mismatch(es); run `cargo xtask bump <version>`:\n  {}",
            errors.len(),
            errors.join("\n  ")
        );
    }
    Ok(expected)
}

/// Every workspace member must inherit the workspace version.
fn member_manifest_errors(root: &Path) -> Result<Vec<String>> {
    let mut manifests = vec![root.join("xtask/Cargo.toml")];
    let crates = root.join("crates");
    if crates.is_dir() {
        for entry in
            std::fs::read_dir(&crates).with_context(|| format!("read {}", crates.display()))?
        {
            let path = entry?.path().join("Cargo.toml");
            if path.is_file() {
                manifests.push(path);
            }
        }
    }
    manifests.sort();
    let mut errors = Vec::new();
    for path in manifests.iter().filter(|p| p.is_file()) {
        let text = read(path)?;
        let inherits = toml_lines(&text)
            .any(|l| l.table == Some("package") && l.key() == Some("version.workspace"));
        if !inherits {
            let rel = path.strip_prefix(root).unwrap_or(path);
            errors.push(format!(
                "{}: [package] must use `version.workspace = true`",
                rel.display()
            ));
        }
    }
    Ok(errors)
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

fn load(root: &Path) -> Result<Vec<VersionFile>> {
    let mut out = Vec::new();
    for &(rel, kind) in FILES {
        let text = read(&root.join(rel))?;
        let sites = match kind {
            Kind::RootManifest => root_manifest_sites(&text),
            Kind::CargoLock => cargo_lock_sites(&text),
            Kind::TomlKey(table) => toml_lines(&text)
                .filter(|l| l.table == table && l.key() == Some("version"))
                .filter_map(|l| l.string_value_span())
                .map(|span| Site {
                    label: format!("[{}] version", table.unwrap_or("root")),
                    span,
                })
                .take(1)
                .collect(),
            Kind::Json(paths) => paths
                .iter()
                .map(|path| {
                    json_string_span(&text, path)
                        .map(|span| Site {
                            label: format!("\"{}\"", path.join(".")),
                            span,
                        })
                        .with_context(|| format!("{rel}: no string at {path:?}"))
                })
                .collect::<Result<_>>()?,
        };
        if sites.is_empty() {
            bail!("{rel}: found no version line");
        }
        out.push(VersionFile { rel, text, sites });
    }
    Ok(out)
}

fn root_manifest_sites(text: &str) -> Vec<Site> {
    let mut sites = Vec::new();
    for line in toml_lines(text) {
        match (line.table, line.key()) {
            (Some("workspace.package"), Some("version")) => {
                if let Some(span) = line.string_value_span() {
                    // The reference version is always sites[0].
                    sites.insert(
                        0,
                        Site {
                            label: "[workspace.package] version".into(),
                            span,
                        },
                    );
                }
            }
            (Some("workspace.dependencies"), Some(key)) if line.text.contains("path =") => {
                if let Some(span) = inline_version_span(line.text) {
                    sites.push(Site {
                        label: format!("[workspace.dependencies] {key}"),
                        span: line.offset + span.start..line.offset + span.end,
                    });
                }
            }
            _ => {}
        }
    }
    sites
}

/// Packages without a `source` line are the lockfile's path packages.
fn cargo_lock_sites(text: &str) -> Vec<Site> {
    let mut sites = Vec::new();
    let mut name = None;
    let mut version = None;
    let mut has_source = false;
    let mut flush =
        |name: &mut Option<String>, version: &mut Option<Range<usize>>, has_source: &mut bool| {
            if let (Some(n), Some(v), false) = (name.take(), version.take(), *has_source) {
                sites.push(Site {
                    label: format!("package {n}"),
                    span: v,
                });
            }
            *has_source = false;
        };
    for line in toml_lines(text) {
        if line.text.trim() == "[[package]]" {
            flush(&mut name, &mut version, &mut has_source);
            continue;
        }
        if line.table != Some("[package]") {
            continue;
        }
        match line.key() {
            Some("name") => {
                name = line.string_value_span().map(|s| text[s].to_owned());
            }
            Some("version") => version = line.string_value_span(),
            Some("source") => has_source = true,
            _ => {}
        }
    }
    flush(&mut name, &mut version, &mut has_source);
    sites
}

struct TomlLine<'a> {
    /// Current table header without brackets (`[[x]]` is reported as `[x]`).
    table: Option<&'a str>,
    text: &'a str,
    offset: usize,
}

impl TomlLine<'_> {
    fn key(&self) -> Option<&str> {
        let trimmed = self.text.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with('[') {
            return None;
        }
        let (key, _) = trimmed.split_once('=')?;
        Some(key.trim())
    }

    /// Absolute span of a `key = "value"` line's value, quotes excluded.
    fn string_value_span(&self) -> Option<Range<usize>> {
        let eq = self.text.find('=')?;
        let rest = &self.text[eq + 1..];
        let open = eq + 1 + rest.find('"')?;
        let close = open + 1 + self.text[open + 1..].find('"')?;
        Some(self.offset + open + 1..self.offset + close)
    }
}

fn toml_lines(text: &str) -> impl Iterator<Item = TomlLine<'_>> {
    let mut table: Option<&str> = None;
    let mut offset = 0;
    text.split_inclusive('\n').map(move |raw| {
        let line = raw.trim_end_matches(['\n', '\r']);
        let start = offset;
        offset += raw.len();
        let trimmed = line.trim();
        if trimmed.starts_with("[[") && trimmed.ends_with("]]") {
            // Keep the array-of-tables distinct from a plain table of the same name.
            table = Some(&trimmed[1..trimmed.len() - 1]);
        } else if let Some(inner) = trimmed.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            table = Some(inner.trim());
        }
        TomlLine {
            table,
            text: line,
            offset: start,
        }
    })
}

/// Span (relative to `line`) of `version = "..."` inside an inline table.
fn inline_version_span(line: &str) -> Option<Range<usize>> {
    let key = line.find("version")?;
    let after = &line[key + "version".len()..];
    let eq = after.find('=')?;
    if !after[..eq].trim().is_empty() {
        return None;
    }
    let open = key + "version".len() + eq + 1 + after[eq + 1..].find('"')?;
    let close = open + 1 + line[open + 1..].find('"')?;
    Some(open + 1..close)
}

/// Byte span (quotes excluded) of the string value at an object-key path.
/// A minimal scanner: enough for package manifests, not a validator.
fn json_string_span(text: &str, path: &[&str]) -> Option<Range<usize>> {
    struct Scan<'a> {
        b: &'a [u8],
        i: usize,
        target: &'a [&'a str],
        stack: Vec<String>,
        found: Option<Range<usize>>,
    }
    impl Scan<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        /// Returns the span between the quotes; `self.i` ends after the closing quote.
        fn string(&mut self) -> Option<Range<usize>> {
            if self.b.get(self.i) != Some(&b'"') {
                return None;
            }
            let start = self.i + 1;
            self.i = start;
            while self.i < self.b.len() {
                match self.b[self.i] {
                    b'\\' => self.i += 2,
                    b'"' => {
                        self.i += 1;
                        return Some(start..self.i - 1);
                    }
                    _ => self.i += 1,
                }
            }
            None
        }
        fn value(&mut self) -> Option<()> {
            self.ws();
            match *self.b.get(self.i)? {
                b'{' => {
                    self.i += 1;
                    loop {
                        self.ws();
                        if self.b.get(self.i) == Some(&b'}') {
                            self.i += 1;
                            return Some(());
                        }
                        let key = self.string()?;
                        let key = std::str::from_utf8(&self.b[key]).ok()?.to_owned();
                        self.ws();
                        if self.b.get(self.i) != Some(&b':') {
                            return None;
                        }
                        self.i += 1;
                        self.stack.push(key);
                        self.value()?;
                        self.stack.pop();
                        self.ws();
                        match self.b.get(self.i)? {
                            b',' => self.i += 1,
                            b'}' => {}
                            _ => return None,
                        }
                    }
                }
                b'[' => {
                    self.i += 1;
                    loop {
                        self.ws();
                        if self.b.get(self.i) == Some(&b']') {
                            self.i += 1;
                            return Some(());
                        }
                        // Array elements never match an object-key path.
                        self.stack.push("\0[]".into());
                        self.value()?;
                        self.stack.pop();
                        self.ws();
                        match self.b.get(self.i)? {
                            b',' => self.i += 1,
                            b']' => {}
                            _ => return None,
                        }
                    }
                }
                b'"' => {
                    let span = self.string()?;
                    if self.found.is_none()
                        && self
                            .stack
                            .iter()
                            .map(String::as_str)
                            .eq(self.target.iter().copied())
                    {
                        self.found = Some(span);
                    }
                    Some(())
                }
                _ => {
                    while self.i < self.b.len() && !b",}] \t\r\n".contains(&self.b[self.i]) {
                        self.i += 1;
                    }
                    Some(())
                }
            }
        }
    }
    let mut scan = Scan {
        b: text.as_bytes(),
        i: 0,
        target: path,
        stack: Vec::new(),
        found: None,
    };
    scan.value()?;
    scan.found
}

/// SemVer 2.0 `MAJOR.MINOR.PATCH[-PRERELEASE]`; build metadata is rejected
/// because the tag `v{V}` must round-trip through every manifest.
fn validate_version(v: &str) -> Result<()> {
    let (core, pre) = match v.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (v, None),
    };
    let numeric = |s: &str| {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'))
    };
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = parts.len() == 3 && parts.iter().all(|p| numeric(p));
    let pre_ok = pre.is_none_or(|pre| {
        pre.split('.').all(|id| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && (!id.bytes().all(|b| b.is_ascii_digit()) || numeric(id))
        })
    });
    if !core_ok || !pre_ok {
        bail!("{v:?} is not a version of the form MAJOR.MINOR.PATCH[-PRERELEASE]");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copies every version-carrying file (and the member manifests) into a
    /// temp workspace.
    fn temp_copy() -> tempfile::TempDir {
        let root = workspace_root();
        let dir = tempfile::tempdir().expect("tempdir");
        let mut rels: Vec<PathBuf> = FILES.iter().map(|(rel, _)| PathBuf::from(rel)).collect();
        rels.push("xtask/Cargo.toml".into());
        for entry in std::fs::read_dir(root.join("crates")).expect("crates/") {
            let name = entry.expect("entry").file_name();
            let rel = Path::new("crates").join(name).join("Cargo.toml");
            if root.join(&rel).is_file() {
                rels.push(rel);
            }
        }
        for rel in rels {
            let dest = dir.path().join(&rel);
            std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
            std::fs::copy(root.join(&rel), dest).unwrap();
        }
        dir
    }

    #[test]
    fn the_repository_is_in_lockstep() {
        check_versions(&workspace_root(), None).expect("versions agree");
    }

    #[test]
    fn bump_rewrites_every_site_and_keeps_formatting() {
        let dir = temp_copy();
        let before = check_versions(dir.path(), None).expect("starts in lockstep");
        let original: Vec<String> = FILES
            .iter()
            .map(|(rel, _)| std::fs::read_to_string(dir.path().join(rel)).unwrap())
            .collect();

        let changed = bump(dir.path(), "7.8.9-rc.1").expect("bump");
        assert_eq!(changed.len(), FILES.len(), "every file carries the version");
        assert_eq!(
            check_versions(dir.path(), Some("v7.8.9-rc.1")).unwrap(),
            "7.8.9-rc.1"
        );
        assert!(check_versions(dir.path(), Some(&format!("v{before}"))).is_err());

        // Only version strings changed: bumping back restores the bytes.
        bump(dir.path(), &before).expect("bump back");
        for ((rel, _), text) in FILES.iter().zip(&original) {
            assert_eq!(
                &std::fs::read_to_string(dir.path().join(rel)).unwrap(),
                text,
                "{rel}"
            );
        }

        // Re-read with real parsers.
        bump(dir.path(), "7.8.9").unwrap();
        let pkg: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("editors/vscode/package-lock.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(pkg["version"], "7.8.9");
        assert_eq!(pkg["packages"][""]["version"], "7.8.9");
        let lock = std::fs::read_to_string(dir.path().join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"suspect-low\"\nversion = \"7.8.9\""));
        let manifest = std::fs::read_to_string(dir.path().join("Cargo.toml")).unwrap();
        assert!(
            manifest
                .contains("suspect-low = { path = \"crates/suspect-low\", version = \"7.8.9\" }")
        );
        // External packages keep their own versions.
        assert!(lock.contains("name = \"serde\""));
        assert!(!lock.contains("name = \"serde\"\nversion = \"7.8.9\""));
    }

    #[test]
    fn check_reports_each_drifted_line() {
        let dir = temp_copy();
        let path = dir.path().join("editors/zed/extension.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let version = check_versions(dir.path(), None).unwrap();
        std::fs::write(
            &path,
            text.replacen(
                &format!("version = \"{version}\""),
                "version = \"9.9.9\"",
                1,
            ),
        )
        .unwrap();
        let err = check_versions(dir.path(), None).unwrap_err().to_string();
        assert!(err.contains("editors/zed/extension.toml"), "{err}");
        assert!(err.contains("1 version mismatch"), "{err}");
    }

    #[test]
    fn version_syntax() {
        for ok in ["0.1.0", "1.20.3", "0.2.0-rc.1", "1.0.0-alpha-2.x"] {
            validate_version(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in [
            "",
            "1.2",
            "v1.2.3",
            "01.2.3",
            "1.2.3+build",
            "1.2.3-",
            "1.2.3-01",
            "1.2.3-a..b",
        ] {
            assert!(validate_version(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_scanner_finds_nested_keys_only() {
        let text = r#"{"a": [{"version": "x"}], "packages": {"": {"version": "1.0.0"}, "node_modules/x": {"version": "2"}}, "version": "3"}"#;
        assert_eq!(&text[json_string_span(text, &["version"]).unwrap()], "3");
        assert_eq!(
            &text[json_string_span(text, &["packages", "", "version"]).unwrap()],
            "1.0.0"
        );
        assert!(json_string_span(text, &["missing"]).is_none());
    }
}
