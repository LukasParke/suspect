//! Instrument the canonical persistent Session. See docs/SDK-SESSION-PERFORMANCE.md.
//! All edits and writes are confined to a newly created private benchmark tree.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_codegen::OutFile;
use suspect_codegen::backend::{Backend, TargetConfig};
use suspect_codegen::generation_session::{Session, SessionConfig, SessionOutput, Stats};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[path = "sdk_session_bench/attribution.rs"]
mod attribution;

const FORMAT: &str = "suspect-sdk-session-bench-v2";
const OWNER: &str = "suspect-sdk:session-performance";
const DOC_PROBE: &str = "SDK session performance documentation probe.";
const PROPERTY_PROBE: &str = "sdkSessionProbe";
const SWIFT_PACKAGE: &str = "BenchmarkSDK";
const SWIFT_MODULE_PROBE: &str = "BenchmarkModule";

struct CountingAllocator;
static MEASURING: AtomicBool = AtomicBool::new(false);
static OBSERVING: AtomicBool = AtomicBool::new(true);
static ATTRIBUTING: AtomicBool = AtomicBool::new(false);
static ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);

fn allocated(pointer: *mut u8, bytes: usize) {
    if !pointer.is_null() && MEASURING.load(Ordering::Relaxed) {
        ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

// SAFETY: every operation delegates unchanged to System, including on allocation
// failure. The counters allocate nothing and are process-wide atomics.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        allocated(pointer, layout.size());
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        allocated(pointer, layout.size());
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        allocated(pointer, size);
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Parser)]
#[command(about = "Measure canonical SDK session refreshes and enforce functional invariants")]
struct Options {
    #[arg(long)]
    spec: PathBuf,
    /// A new directory below this repository's target directory. Parent must exist.
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value = "custom")]
    fixture: String,
    #[arg(long)]
    operation_id: Vec<String>,
    /// Explicit profiles; adding a backend also requires its docs-only oracle.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "typescript-http,rust-http,python-http,go-http,swift-http"
    )]
    targets: Vec<String>,
    /// Independent fresh-session cycles, each followed by warm/edit/revert refreshes.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u16).range(1..=10000))]
    iterations: u16,
    /// Entire cycles, checked but omitted from timing samples.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u16).range(0..=100))]
    warmups: u16,
    #[arg(long, default_value_t = 4)]
    cache_entries: usize,
    #[arg(long, default_value_t = 268_435_456)]
    cache_bytes: usize,
    /// One checked cycle and configuration probes, without timing/allocation observations.
    #[arg(long)]
    functional_only: bool,
    /// Record CPU, block IO, fault and scheduler counters outside the timed interval.
    #[arg(long, conflicts_with = "functional_only")]
    attribution: bool,
    /// Fix scenario position, or retain the original rotating edit/revert workload.
    #[arg(long, value_parser = ["rotating", "fixed"], default_value = "rotating")]
    schedule: String,
}

#[derive(Serialize)]
struct Phase {
    ms: f64,
    allocation_calls: u64,
    allocated_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    resources: Option<attribution::Delta>,
}

fn measure<T>(run: impl FnOnce() -> Result<T>) -> Result<(T, Phase)> {
    if !OBSERVING.load(Ordering::Relaxed) {
        return Ok((
            run()?,
            Phase {
                ms: 0.0,
                allocation_calls: 0,
                allocated_bytes: 0,
                resources: None,
            },
        ));
    }
    let before = ATTRIBUTING
        .load(Ordering::Relaxed)
        .then(attribution::Snapshot::now)
        .transpose()?;
    ALLOCATION_CALLS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    MEASURING.store(true, Ordering::Relaxed);
    let start = Instant::now();
    let result = run();
    let elapsed = start.elapsed();
    MEASURING.store(false, Ordering::Relaxed);
    let phase = Phase {
        ms: elapsed.as_secs_f64() * 1000.0,
        allocation_calls: ALLOCATION_CALLS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        resources: before
            .map(|before| attribution::Snapshot::now()?.since(before))
            .transpose()?,
    };
    Ok((result?, phase))
}

fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Fingerprint {
    path: String,
    bytes: usize,
    sha256: String,
}

fn fingerprint(path: impl Into<String>, bytes: &[u8]) -> Fingerprint {
    Fingerprint {
        path: path.into(),
        bytes: bytes.len(),
        sha256: hash(bytes),
    }
}

fn compile(entry: &Path) -> Result<Arc<Contract>> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(entry.parent().context("entry parent")?)
            .build()?,
    );
    Ok(Arc::new(Contract::from_workspace(
        &workspace,
        &Uri::from_path(entry)?,
    )?))
}

struct Inputs {
    originals: BTreeMap<PathBuf, Vec<u8>>,
    original_root: PathBuf,
    private_root: PathBuf,
    entry: PathBuf,
    normalized: Vec<Fingerprint>,
}

impl Inputs {
    fn prepare(entry: &Path, private: &Path) -> Result<Self> {
        let entry = entry.canonicalize()?;
        // Discovery is untimed. Only discovered, local closure members are copied.
        let discovered = compile(&entry)?;
        let originals: BTreeMap<_, _> = discovered
            .documents()
            .map(|(uri, _)| {
                let path = uri
                    .as_path()
                    .with_context(|| format!("non-file input: {uri}"))?;
                Ok((path.clone(), fs::read(path)?))
            })
            .collect::<Result<_>>()?;
        let common = entry
            .parent()
            .context("entry parent")?
            .ancestors()
            .find(|root| originals.keys().all(|path| path.starts_with(root)))
            .context("closure has no common directory")?
            .to_path_buf();
        fs::create_dir(private)?;
        let private = private.canonicalize()?;
        for (path, bytes) in &originals {
            let copied = private.join(path.strip_prefix(&common)?);
            fs::create_dir_all(copied.parent().context("copy parent")?)?;
            fs::write(copied, bytes)?;
        }
        let copied_entry = private.join(entry.strip_prefix(&common)?);
        let copied = compile(&copied_entry)?;
        let expected_paths: BTreeSet<_> = originals
            .keys()
            .map(|path| private.join(path.strip_prefix(&common).expect("common root")))
            .collect();
        let actual_paths: BTreeSet<_> = copied
            .documents()
            .map(|(uri, _)| {
                uri.as_path()
                    .context("private closure must contain only local files")
            })
            .collect::<Result<_>>()?;
        ensure!(
            actual_paths == expected_paths,
            "private closure escaped the copy or changed membership"
        );
        // Canonical JSON in the original filenames makes edits independent of YAML
        // formatting. Cold samples measure this explicitly fingerprinted input form.
        let mut normalized = Vec::new();
        for (uri, document) in copied.documents() {
            let path = uri.as_path().context("private document path")?;
            let bytes = serde_json::to_vec_pretty(document)?;
            fs::write(&path, &bytes)?;
            normalized.push(fingerprint(
                path.strip_prefix(&private)?.to_string_lossy(),
                &bytes,
            ));
        }
        normalized.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Self {
            originals,
            original_root: common,
            private_root: private,
            entry: copied_entry,
            normalized,
        })
    }

    fn original_fingerprints(&self) -> Result<Vec<Fingerprint>> {
        self.originals
            .iter()
            .map(|(path, bytes)| {
                Ok(fingerprint(
                    path.strip_prefix(&self.original_root)?.to_string_lossy(),
                    bytes,
                ))
            })
            .collect()
    }

    fn verify(&self) -> Result<()> {
        for (path, bytes) in &self.originals {
            ensure!(
                fs::read(path)? == *bytes,
                "original source changed during measurement: {}",
                path.display()
            );
        }
        for expected in &self.normalized {
            let current = fs::read(self.private_root.join(&expected.path))?;
            ensure!(
                fingerprint(&expected.path, &current) == *expected,
                "private edit was not reverted: {}",
                expected.path
            );
        }
        Ok(())
    }
}

struct Edit {
    name: &'static str,
    source: SourceId,
    before: Vec<u8>,
    after: Vec<u8>,
    expected: Arc<Vec<OutFile>>,
}

impl Edit {
    fn write(&self, changed: bool) -> Result<()> {
        let path = self.source.document().as_path().context("edit document")?;
        let (expected, next) = if changed {
            (&self.before, &self.after)
        } else {
            (&self.after, &self.before)
        };
        ensure!(
            fs::read(&path)? == *expected,
            "unexpected private bytes before {}",
            self.name
        );
        fs::write(path, next)?;
        Ok(())
    }
}

fn selected(contract: &Contract, names: &[String]) -> Result<Vec<SourceId>> {
    let available: Vec<_> = contract.operations().collect();
    let mut ids = Vec::new();
    if names.is_empty() {
        ids.extend(available.iter().map(|operation| operation.source().clone()));
    } else {
        for name in names {
            let matching: Vec<_> = available
                .iter()
                .filter(|operation| operation.operation_id() == Some(name.as_str()))
                .collect();
            ensure!(
                matching.len() == 1,
                "operationId {name:?} must match exactly one operation"
            );
            ids.push(matching[0].source().clone());
        }
    }
    ids.sort();
    ids.dedup();
    ensure!(
        !ids.is_empty(),
        "HTTP refresh benchmarks require selected operations"
    );
    Ok(ids)
}

fn schema_source(contract: &Contract, operations: &[SourceId]) -> Result<SourceId> {
    let mut roots = Vec::new();
    for operation in contract
        .operations()
        .filter(|operation| operations.contains(operation.source()))
    {
        if let Some(body) = operation.request_body() {
            roots.extend(
                body.content()
                    .into_iter()
                    .filter_map(|media| media.schema())
                    .map(|schema| schema.id().clone()),
            );
        }
        for response in operation.responses() {
            roots.extend(
                response
                    .content()
                    .into_iter()
                    .filter_map(|media| media.schema())
                    .map(|schema| schema.id().clone()),
            );
        }
    }
    contract
        .reachable_from(&roots)
        .into_iter()
        .find(|id| {
            contract
                .source(id)
                .is_some_and(|value| value["type"] == "object" && value.get("$ref").is_none())
        })
        .context("schema edit requires a reachable object schema")
}

fn prepare_edit(
    name: &'static str,
    source: SourceId,
    contract: &Contract,
    entry: &Path,
    config: &SessionConfig,
    mutate: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<Edit> {
    let path = source.document().as_path().context("edit path")?;
    let before = fs::read(&path)?;
    let mut document = contract
        .document(source.document())
        .context("edit document")?
        .clone();
    mutate(
        document
            .pointer_mut(source.pointer())
            .context("edit pointer")?,
    )?;
    let after = serde_json::to_vec_pretty(&document)?;
    ensure!(before != after, "{name} did not modify the private source");
    fs::write(&path, &after)?;
    // Independent fresh Session oracle, excluded from samples and allocation counts.
    let oracle = Session::new(entry, config.clone())?.generate();
    fs::write(&path, &before)?;
    Ok(Edit {
        name,
        source,
        before,
        after,
        expected: oracle?.files,
    })
}

fn json_without_docs(text: &str, source: &SourceId, examples: bool) -> Result<Value> {
    let mut value: Value = serde_json::from_str(text)?;
    if examples {
        for finding in value["diagnostics"]
            .as_array_mut()
            .context("example diagnostics")?
        {
            finding
                .as_object_mut()
                .context("example finding")?
                .remove("range");
        }
    } else {
        let mut count = 0;
        for operation in value["operations"]
            .as_array_mut()
            .context("HTTP manifest")?
        {
            if operation["source"]["document"] == source.document().as_str()
                && operation["source"]["pointer"] == source.pointer()
            {
                let object = operation.as_object_mut().context("manifest operation")?;
                object.remove("description");
                object.remove("descriptionText");
                object.remove("hasSourceDescription");
                object.remove("provenance");
                object.remove("source");
                object.remove("sourceOperationId");
                object.remove("wire");
                object.remove("attribution");
                count += 1;
            }
        }
        ensure!(
            count == 1,
            "edited operation missing or duplicated in manifest"
        );
    }
    Ok(value)
}

fn python_without_operation_doc(text: &str, source: &SourceId) -> Result<String> {
    let suffix = format!(" Source: {}#{}", source.document(), source.pointer());
    let mut lines = Vec::new();
    let mut after_def = false;
    let mut count = 0;
    for line in text.lines() {
        let decoded = if after_def {
            serde_json::from_str::<String>(line.trim()).ok()
        } else {
            None
        };
        if decoded.is_some_and(|value| value.ends_with(&suffix)) {
            lines.push("        \"<operation documentation>\"");
            count += 1;
        } else {
            lines.push(line);
        }
        after_def = line.starts_with("    def ") || line.starts_with("    async def ");
    }
    ensure!(
        count == 2,
        "expected source-bound sync and async Python method docstrings"
    );
    Ok(lines.join("\n"))
}

/// Normalize only the comments immediately above the source-bound Swift method.
/// Its metadata and every executable byte remain in the comparison.
fn swift_without_operation_doc(text: &str, source: &SourceId, changed: bool) -> Result<String> {
    let lines: Vec<_> = text.lines().collect();
    let suffix = format!(" Source: {}#{}.", source.document(), source.pointer());
    let indices: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("    /// ") && line.contains(&suffix))
        .map(|(index, _)| index)
        .collect();
    ensure!(
        indices.len() == 1,
        "expected one source-bound Swift operation doc block"
    );
    let metadata = indices[0];
    ensure!(
        lines
            .get(metadata + 1)
            .is_some_and(|line| line.starts_with("    public func ")),
        "Swift operation metadata is not bound to a method"
    );
    let mut start = metadata;
    while start > 0 && lines[start - 1].starts_with("    ///") {
        start -= 1;
    }
    ensure!(
        start > 0 && lines[start - 1] == "extension Client {",
        "ambiguous Swift operation documentation"
    );
    if changed {
        ensure!(
            lines[start..metadata] == [format!("    /// {DOC_PROBE}")],
            "Swift operation documentation is stale"
        );
    }
    Ok(lines[..start]
        .iter()
        .chain(lines[metadata..].iter())
        .copied()
        .collect::<Vec<_>>()
        .join("\n"))
}

fn swift_coverage_without_description(text: &str, source: &SourceId) -> Result<Value> {
    let mut value: Value = serde_json::from_str(text)?;
    let location = format!("{}#{}", source.document(), source.pointer());
    value["missingOperationDescriptions"]
        .as_array_mut()
        .context("Swift description coverage")?
        .retain(|item| item != &json!(location));
    Ok(value)
}

fn swift_root(files: &[OutFile]) -> Result<&str> {
    let manifests: Vec<_> = files
        .iter()
        .filter_map(|file| file.path.strip_suffix("Package.swift"))
        .collect();
    ensure!(manifests.len() == 1, "expected one Swift Package.swift");
    Ok(manifests[0])
}

fn swift_identity(files: &[OutFile], module: &str) -> Result<()> {
    let root = swift_root(files)?;
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == format!("{root}sdk-manifest.json"))
            .context("Swift SDK manifest")?
            .content,
    )?;
    ensure!(
        manifest["package"] == SWIFT_PACKAGE
            && manifest["module"] == module
            && manifest["version"] == "0.0.0",
        "Swift package/module identity is stale"
    );
    ensure!(
        files
            .iter()
            .any(|file| file.path == format!("{root}Sources/{module}/Operations.swift")),
        "Swift module source path is missing"
    );
    Ok(())
}

fn go_without_operation_prose(text: &str, description: &str) -> (String, usize) {
    let plain = format!(
        "// {}",
        description.replace(['\n', '\r'], " ").replace("*/", "* /")
    );
    let native = format!(
        "//\t{}",
        description.replace(['\n', '\r', '\u{2028}', '\u{2029}'], " ")
    );
    let mut count = 0;
    let normalized = text
        .lines()
        .map(|line| {
            if line == plain || line == native {
                count += 1;
                "// <operation documentation>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    (normalized, count)
}

fn bindings_without_operation_prose(text: &str, source: &SourceId) -> Result<Value> {
    let mut value: Value = serde_json::from_str(text)?;
    let mut count = 0;
    for symbol in value["symbols"]
        .as_array_mut()
        .context("documentation symbols")?
    {
        if symbol["source"]["document"] == source.document().as_str()
            && symbol["source"]["pointer"] == source.pointer()
        {
            symbol
                .as_object_mut()
                .context("documentation symbol")?
                .remove("description");
            count += 1;
        }
    }
    ensure!(
        count > 0,
        "edited operation is absent from documentation bindings"
    );
    Ok(value)
}

fn rst_literal(text: &str) -> String {
    let normalized = text.replace(
        [
            '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
        ],
        "\n",
    );
    format!(
        ".. code-block:: text\n\n{}\n",
        normalized
            .lines()
            .map(|line| format!("   {line}\n"))
            .collect::<String>()
    )
}

/// Compare executable syntax without erasing quoted literals. Ambiguous JS
/// regex/template interpolation is rejected rather than treated as prose.
fn without_doc_comments(path: &str, text: &str) -> Result<String> {
    let rust = path.ends_with(".rs");
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if rust && bytes[i] == b'r' {
            let mut quote = i + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let closing = format!("\"{}", "#".repeat(quote - i - 1));
                let end = text[quote + 1..]
                    .find(&closing)
                    .context("unclosed raw string")?
                    + quote
                    + 1
                    + closing.len();
                out.push_str(&text[i..end]);
                i = end;
                continue;
            }
        }
        let rust_char = rust
            && bytes[i] == b'\''
            && (bytes.get(i + 1) == Some(&b'\\')
                || text
                    .get(i + 1..)
                    .and_then(|rest| rest.chars().next())
                    .is_some_and(|ch| bytes.get(i + 1 + ch.len_utf8()) == Some(&b'\'')));
        if bytes[i] == b'"' || (!rust && matches!(bytes[i], b'\'' | b'`')) || rust_char {
            let delimiter = bytes[i];
            let start = i;
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if !rust && delimiter == b'`' && bytes[i..].starts_with(b"${") {
                    bail!("interpolated template requires native syntax comparison");
                }
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == delimiter {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            ensure!(closed, "unclosed quoted literal");
            out.push_str(&text[start..i]);
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            let end = text[i..]
                .find('\n')
                .map_or(bytes.len(), |offset| i + offset);
            if rust && (bytes[i..].starts_with(b"///") || bytes[i..].starts_with(b"//!")) {
                out.push(' ');
            } else {
                out.push_str(&text[i..end]);
                out.push('\n');
            }
            i = end;
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let doc = bytes[i..].starts_with(b"/**") || rust && bytes[i..].starts_with(b"/*!");
            let start = i;
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if rust && bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            ensure!(depth == 0, "unclosed comment");
            if doc {
                out.push(' ');
            } else {
                out.push_str(&text[start..i]);
            }
            continue;
        }
        if !rust && bytes[i] == b'/' {
            bail!("regexp/division requires native syntax comparison");
        }
        let ch = text[i..].chars().next().context("character boundary")?;
        if ch.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    Ok(out.trim().to_owned())
}

fn docs_only(
    before: &[OutFile],
    after: &[OutFile],
    source: &SourceId,
    original: &str,
) -> Result<()> {
    let old: BTreeMap<_, _> = before
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    let new: BTreeMap<_, _> = after
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    ensure!(
        old.keys().eq(new.keys()),
        "docs-only edit added or removed artifacts"
    );
    let swift = after
        .iter()
        .any(|file| file.path.ends_with("Package.swift"));
    let swift_prefix = if swift {
        Some(swift_root(after)?)
    } else {
        None
    };
    for (path, text) in &new {
        if old[path] == *text {
            continue;
        }
        if path.ends_with("/http-manifest.json") {
            ensure!(text.contains(DOC_PROBE), "docs probe missing from {path}");
            ensure!(
                json_without_docs(old[path], source, false)?
                    == json_without_docs(text, source, false)?,
                "docs edit changed semantic manifest: {path}"
            );
        } else if path.ends_with("/examples.json") {
            ensure!(
                json_without_docs(old[path], source, true)?
                    == json_without_docs(text, source, true)?,
                "docs edit changed examples: {path}"
            );
        } else if (path.starts_with("typescript/") && path.ends_with(".ts"))
            || (path.starts_with("rust/") && path.ends_with(".rs"))
        {
            ensure!(
                without_doc_comments(path, old[path])? == without_doc_comments(path, text)?,
                "docs edit changed executable syntax: {path}"
            );
            ensure!(
                text.contains(DOC_PROBE),
                "source prose did not reach changed native comments: {path}"
            );
        } else if *path == "typescript/http.md" || *path == "rust/README.md" {
            ensure!(
                text.contains(DOC_PROBE),
                "native source documentation is stale"
            );
        } else if path.starts_with("python/") && path.ends_with("/_client.py") {
            ensure!(text.contains(DOC_PROBE), "Python docstring is stale");
            ensure!(
                python_without_operation_doc(old[path], source)?
                    == python_without_operation_doc(text, source)?,
                "docs edit changed executable Python"
            );
        } else if *path == "go/operations.go" || *path == "go/doc.go" {
            let before = go_without_operation_prose(old[path], original);
            let after = go_without_operation_prose(text, DOC_PROBE);
            ensure!(
                before.1 > 0 && before == after,
                "docs edit changed executable Go or ambiguous source comments: {path}"
            );
        } else if *path == "go/docs/source-bindings.json"
            || *path == "python/docs/source-bindings.json"
        {
            ensure!(
                text.contains(DOC_PROBE)
                    && bindings_without_operation_prose(old[path], source)?
                        == bindings_without_operation_prose(text, source)?,
                "docs edit changed semantic documentation bindings: {path}"
            );
        } else if *path == "go/docs/api.rst" || *path == "python/docs/api.rst" {
            ensure!(
                !original.is_empty() && old[path].contains(&rst_literal(original)),
                "source prose literal missing from {path}"
            );
            ensure!(
                old[path].replace(&rst_literal(original), &rst_literal(DOC_PROBE)) == *text,
                "docs edit changed unrelated Sphinx content: {path}"
            );
        } else if *path == "go/README.md" {
            ensure!(text.contains(DOC_PROBE), "Go README is stale");
        } else if swift_prefix
            .is_some_and(|root| *path == format!("{root}Sources/{SWIFT_PACKAGE}/Operations.swift"))
        {
            ensure!(
                swift_without_operation_doc(old[path], source, false)?
                    == swift_without_operation_doc(text, source, true)?,
                "docs edit changed executable Swift"
            );
        } else if swift_prefix.is_some_and(|root| {
            *path
                == format!(
                    "{root}Sources/{SWIFT_PACKAGE}/{SWIFT_PACKAGE}.docc/OperationReference.md"
                )
        }) {
            ensure!(
                text.contains(DOC_PROBE),
                "Swift DocC operation reference is stale"
            );
        } else if swift_prefix.is_some_and(|root| *path == format!("{root}example-coverage.json")) {
            ensure!(
                swift_coverage_without_description(old[path], source)?
                    == swift_coverage_without_description(text, source)?,
                "docs edit changed Swift examples/coverage outside the selected description"
            );
        } else {
            bail!(
                "docs edit changed unrelated artifact (new backends need an explicit oracle): {path}"
            );
        }
    }
    for (manifest, page) in [
        ("typescript/http-manifest.json", "typescript/operations.ts"),
        ("rust/http-manifest.json", "rust/README.md"),
    ] {
        if let Some(text) = new.get(manifest) {
            let value: Value = serde_json::from_str(text)?;
            let matching: Vec<_> = value["operations"]
                .as_array()
                .context("native operation manifest")?
                .iter()
                .filter(|item| {
                    item["source"]["document"] == source.document().as_str()
                        && item["source"]["pointer"] == source.pointer()
                })
                .collect();
            ensure!(
                matching.len() == 1 && matching[0]["descriptionText"] == DOC_PROBE,
                "native operation description is stale: {manifest}"
            );
            ensure!(
                new.get(page).is_some_and(|text| text.contains(DOC_PROBE))
                    && old.get(page) != new.get(page),
                "source prose did not update its native documentation: {page}"
            );
        }
    }
    if let Some(root) = swift_prefix {
        for path in [
            format!("{root}Sources/{SWIFT_PACKAGE}/Operations.swift"),
            format!("{root}Sources/{SWIFT_PACKAGE}/{SWIFT_PACKAGE}.docc/OperationReference.md"),
        ] {
            ensure!(
                new.get(path.as_str())
                    .is_some_and(|text| text.contains(DOC_PROBE))
                    && old.get(path.as_str()) != new.get(path.as_str()),
                "Swift source documentation did not update: {path}"
            );
        }
    }
    for root in ["typescript/", "rust/", "python/", "go/"] {
        if old.keys().any(|path| path.starts_with(root)) {
            ensure!(
                new.iter().any(|(path, text)| path.starts_with(root)
                    && text.contains(DOC_PROBE)
                    && old[path] != *text),
                "{root} documentation did not update"
            );
        }
    }
    Ok(())
}

fn file_hashes(files: &[OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| (file.path.clone(), hash(&file.content)))
        .collect()
}

fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    before
        .keys()
        .chain(after.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|path| before.get(*path) != after.get(*path))
        .cloned()
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    sha256: String,
    modified: SystemTime,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed: (i64, i64),
}

fn disk(root: &Path, output: &SessionOutput) -> Result<BTreeMap<String, Stamp>> {
    fn visit(root: &Path, directory: &Path, found: &mut BTreeMap<String, Stamp>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                !metadata.is_symlink(),
                "unexpected symlink in private artifacts"
            );
            if metadata.is_dir() {
                visit(root, &path, found)?;
                continue;
            }
            ensure!(metadata.is_file(), "non-file artifact");
            #[cfg(unix)]
            use std::os::unix::fs::MetadataExt;
            found.insert(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                Stamp {
                    sha256: hash(fs::read(&path)?),
                    modified: metadata.modified()?,
                    #[cfg(unix)]
                    inode: metadata.ino(),
                    #[cfg(unix)]
                    changed: (metadata.ctime(), metadata.ctime_nsec()),
                },
            );
        }
        Ok(())
    }
    let mut found = BTreeMap::new();
    visit(root, root, &mut found)?;
    let mut expected = file_hashes(&output.files);
    for (path, digest) in &expected {
        ensure!(
            found.get(path).is_some_and(|stamp| &stamp.sha256 == digest),
            "missing or stale artifact: {path}"
        );
    }
    expected.insert(suspect_artifact::OWNERSHIP_MANIFEST.into(), String::new());
    ensure!(
        expected.keys().eq(found.keys()),
        "unexpected/stale files or missing ownership manifest on disk"
    );
    Ok(found)
}

#[derive(Default)]
struct RefreshState {
    session: Option<Session>,
    hashes: BTreeMap<String, String>,
    stamps: BTreeMap<String, Stamp>,
}

fn stats(value: Stats) -> Value {
    json!({"compiles":value.compiles,"renders":value.renders,"cache_hits":value.cache_hits})
}

fn rss_after_bytes() -> Option<u64> {
    // Untimed Linux observation. Not peak RSS, and unavailable on other hosts.
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}

impl RefreshState {
    fn refresh(
        &mut self,
        entry: &Path,
        config: &SessionConfig,
        directory: &Path,
        oracle: &[OutFile],
        expected_delta: Stats,
        reuse: Option<&SessionOutput>,
    ) -> Result<(SessionOutput, Value)> {
        let (output, generate) = measure(|| {
            if self.session.is_none() {
                self.session = Some(Session::new(entry, config.clone())?);
            }
            Ok(self
                .session
                .as_mut()
                .expect("initialized Session")
                .generate()?)
        })?;
        let (_, write) = measure(|| {
            Ok(self
                .session
                .as_ref()
                .expect("Session")
                .write(&output, directory)?)
        })?;
        // Everything below is outside both timed/allocation-counted phases.
        ensure!(
            output.delta == expected_delta,
            "redundant/missing work: expected {expected_delta:?}, got {:?}",
            output.delta
        );
        ensure!(
            *output.files == oracle,
            "cached output differs from independent fresh Session oracle"
        );
        if let Some(reuse) = reuse {
            ensure!(
                Arc::ptr_eq(&reuse.contract, &output.contract)
                    && Arc::ptr_eq(&reuse.files, &output.files),
                "cache hit did not reuse the owned snapshot"
            );
        }
        let hashes = file_hashes(&output.files);
        ensure!(
            output.changed_paths == changed(&self.hashes, &hashes),
            "changed_paths does not equal the complete artifact diff"
        );
        if !self.hashes.is_empty() {
            ensure!(
                output.new_documents.is_empty(),
                "unchanged closure membership reported new documents"
            );
        }
        let next = disk(directory, &output)?;
        let redundant_rewrites = self
            .stamps
            .iter()
            .filter(|(path, previous)| {
                next.get(*path).is_some_and(|current| {
                    current.sha256 == previous.sha256 && current != *previous
                })
            })
            .count();
        ensure!(
            redundant_rewrites == 0,
            "byte-identical artifacts/ownership manifest were rewritten"
        );
        let removed = self
            .hashes
            .keys()
            .filter(|path| !hashes.contains_key(*path))
            .count();
        self.stamps = next;
        self.hashes = hashes;
        let sample = json!({
            "generate":generate,"write":write,
            "refresh_ms":generate.ms + write.ms,
            "allocation_calls":generate.allocation_calls + write.allocation_calls,
            "allocated_bytes":generate.allocated_bytes + write.allocated_bytes,
            "artifact_bytes":output.files.iter().map(|file| file.content.len()).sum::<usize>(),
            "artifact_files":output.files.len(),"artifact_sha256":hash(serde_json::to_vec(&self.hashes)?),
            "delta":stats(output.delta),"stats":stats(output.stats),
            "changed_paths":output.changed_paths,"removed_files":removed,"new_documents":output.new_documents.len(),
            "redundant_rewrites":redundant_rewrites,"fresh_oracle_equal":true,"disk_current":true,
            "rss_after_bytes":if OBSERVING.load(Ordering::Relaxed) { rss_after_bytes() } else { None },
        });
        Ok((output, sample))
    }
}

fn configuration(options: &Options) -> Result<SessionConfig> {
    ensure!(
        options.cache_entries >= 2,
        "edit/revert probes require at least two cache entries"
    );
    let targets = options
        .targets
        .iter()
        .map(|name| {
            let backend: Backend = serde_json::from_value(json!(name))
                .with_context(|| format!("unknown backend {name:?}"))?;
            ensure!(
                [
                    "typescript-http",
                    "rust-http",
                    "python-http",
                    "go-http",
                    "swift-http"
                ]
                .contains(&backend.name()),
                "add the new backend's documentation oracle before benchmarking it"
            );
            Ok(TargetConfig {
                backend,
                package_name: match backend.name() {
                    "go-http" => "example.com/sdk-session-perf",
                    "swift-http" => SWIFT_PACKAGE,
                    _ => "sdk-session-perf",
                }
                .into(),
                package_version: "0.0.0".into(),
                import_name: (backend.name() == "python-http").then(|| "sdk_session_perf".into()),
            })
        })
        .collect::<Result<_>>()?;
    Ok(SessionConfig {
        targets,
        operation_ids: options.operation_id.clone(),
        generation: Default::default(),
        cache_entries: options.cache_entries,
        cache_bytes: options.cache_bytes,
        owner: OWNER.into(),
    })
}

fn main() -> Result<()> {
    let process_start = Instant::now();
    let started_at_ns = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let mut options = Options::parse();
    if options.attribution {
        attribution::Snapshot::now()?;
        ATTRIBUTING.store(true, Ordering::Relaxed);
    }
    if options.functional_only {
        options.iterations = 1;
        options.warmups = 0;
        OBSERVING.store(false, Ordering::Relaxed);
    }
    let config = configuration(&options)?;
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .canonicalize()?;
    let parent = options
        .out
        .parent()
        .context("output parent")?
        .canonicalize()?;
    ensure!(
        parent.starts_with(&target),
        "output must be below {}",
        target.display()
    );
    let root = parent.join(options.out.file_name().context("new output directory")?);
    fs::create_dir(&root).with_context(|| format!("output must be new: {}", root.display()))?;
    let executable = std::env::current_exe()?;
    let binary = fingerprint(executable.to_string_lossy(), &fs::read(&executable)?);
    let inputs = Inputs::prepare(&options.spec, &root.join("private"))?;
    let baseline = Session::new(&inputs.entry, config.clone())?.generate()?;
    let swift_target = config
        .targets
        .iter()
        .position(|target| target.backend.name() == "swift-http");
    if swift_target.is_some() {
        swift_identity(&baseline.files, SWIFT_PACKAGE)?;
    }
    ensure!(
        baseline.contract.documents().count() == inputs.normalized.len(),
        "normalized closure membership changed"
    );
    let operations = selected(&baseline.contract, &config.operation_ids)?;
    let operation_source = operations.first().context("operation edit source")?.clone();
    let original_description = baseline
        .contract
        .source(&operation_source)
        .context("operation")?["description"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let schema = schema_source(&baseline.contract, &operations)?;
    let schema_edit = prepare_edit(
        "schema-edit",
        schema,
        &baseline.contract,
        &inputs.entry,
        &config,
        |schema| {
            if schema.get("properties").is_none() {
                schema["properties"] = json!({});
            }
            ensure!(
                schema["properties"].get(PROPERTY_PROBE).is_none(),
                "schema probe already exists"
            );
            schema["properties"][PROPERTY_PROBE] = json!({"type":"string"});
            Ok(())
        },
    )?;
    let operation_edit = prepare_edit(
        "operation-edit",
        operation_source.clone(),
        &baseline.contract,
        &inputs.entry,
        &config,
        |operation| {
            let responses = operation["responses"]
                .as_object_mut()
                .context("operation responses")?;
            let response = responses
                .iter()
                .find(|(key, _)| key.parse::<u16>().is_ok())
                .context("numeric response")?
                .1
                .clone();
            let status = (400..600)
                .map(|code| code.to_string())
                .find(|code| !responses.contains_key(code))
                .context("unused response status")?;
            responses.insert(status, response);
            Ok(())
        },
    )?;
    let docs_edit = prepare_edit(
        "docs-edit",
        operation_source.clone(),
        &baseline.contract,
        &inputs.entry,
        &config,
        |operation| {
            operation["description"] = json!(DOC_PROBE);
            Ok(())
        },
    )?;
    docs_only(
        &baseline.files,
        &docs_edit.expected,
        &operation_source,
        &original_description,
    )?;
    let edits = [schema_edit, operation_edit, docs_edit];
    for edit in &edits {
        for target in &config.targets {
            let prefix = target.backend.name().trim_end_matches("-http");
            ensure!(
                edit.expected.iter().any(|file| (if prefix == "swift" {
                    file.path.ends_with(".swift")
                } else {
                    file.path.starts_with(&format!("{prefix}/"))
                }) && baseline
                    .files
                    .iter()
                    .find(|old| old.path == file.path)
                    .is_none_or(|old| old.content != file.content)),
                "{} failed to reach {}",
                edit.name,
                target.backend.name()
            );
        }
    }
    let miss = Stats {
        compiles: 1,
        renders: config.targets.len(),
        cache_hits: 0,
    };
    let hit = Stats {
        compiles: 0,
        renders: 0,
        cache_hits: 1,
    };
    let directory = root.join("artifacts");
    let mut samples = Vec::new();
    let mut warmup_samples = Vec::new();
    let mut last = RefreshState::default();
    let mut last_baseline = None;
    for cycle in 0..u32::from(options.warmups) + u32::from(options.iterations) {
        // Release the previous cycle's finite cache outside measured intervals.
        drop(std::mem::take(&mut last));
        drop(last_baseline.take());
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        let mut state = RefreshState::default();
        let mut position = 0;
        let mut record = |scenario: &str, mut sample: Value| {
            sample["scenario"] = json!(scenario);
            sample["cycle"] = json!(cycle);
            sample["position_in_cycle"] = json!(position);
            sample["ordinal"] = json!(u64::from(cycle) * 8 + position);
            sample["observed_at_elapsed_ms"] =
                json!(process_start.elapsed().as_secs_f64() * 1000.0);
            position += 1;
            if cycle < u32::from(options.warmups) {
                warmup_samples.push(sample);
            } else {
                samples.push(sample);
            }
        };
        let (cold, sample) = state.refresh(
            &inputs.entry,
            &config,
            &directory,
            &baseline.files,
            miss,
            None,
        )?;
        ensure!(
            cold.new_documents.len() == inputs.normalized.len(),
            "cold run did not report the complete closure"
        );
        record("cold", sample);
        let (_, sample) = state.refresh(
            &inputs.entry,
            &config,
            &directory,
            &baseline.files,
            hit,
            Some(&cold),
        )?;
        record("warm", sample);
        // Fixed rotation reduces always-first scenario bias; the schedule is versioned.
        for index in 0..edits.len() {
            let rotation = if options.schedule == "rotating" {
                cycle as usize
            } else {
                0
            };
            let edit = &edits[(rotation + index) % edits.len()];
            edit.write(true)?;
            let (_, sample) = state.refresh(
                &inputs.entry,
                &config,
                &directory,
                &edit.expected,
                miss,
                None,
            )?;
            record(edit.name, sample);
            edit.write(false)?;
            let (_, sample) = state.refresh(
                &inputs.entry,
                &config,
                &directory,
                &baseline.files,
                hit,
                Some(&cold),
            )?;
            record(&edit.name.replace("-edit", "-revert"), sample);
        }
        last = state;
        last_baseline = Some(cold);
    }
    // An untimed target-set shrink exercises real obsolete-file removal. A package
    // change is the equivalent configuration probe for a single-target run.
    let mut changed_config = config.clone();
    if changed_config.targets.len() > 1 {
        changed_config.targets.pop();
    } else {
        changed_config.targets[0].package_version = "0.0.1".into();
    }
    let fresh_config = Session::new(&inputs.entry, changed_config.clone())?.generate()?;
    last.session
        .as_mut()
        .context("last Session")?
        .set_config(changed_config.clone())?;
    let (configured, config_sample) = last.refresh(
        &inputs.entry,
        &changed_config,
        &directory,
        &fresh_config.files,
        Stats {
            compiles: 0,
            renders: usize::from(config.targets.len() == 1),
            cache_hits: if config.targets.len() > 1 {
                changed_config.targets.len()
            } else {
                0
            },
        },
        None,
    )?;
    ensure!(
        Arc::ptr_eq(
            &configured.contract,
            &last_baseline
                .as_ref()
                .context("last cold snapshot")?
                .contract
        ),
        "configuration recompiled the unchanged contract"
    );
    if config.targets.len() > 1 {
        ensure!(
            config_sample["removed_files"].as_u64().unwrap_or(0) > 0,
            "target removal left stale artifacts"
        );
    }
    last.session
        .as_mut()
        .context("last Session")?
        .set_config(config.clone())?;
    let (_, config_revert) = last.refresh(
        &inputs.entry,
        &config,
        &directory,
        &baseline.files,
        hit,
        last_baseline.as_ref(),
    )?;
    let module_probe = if let Some(index) = swift_target {
        let mut module_config = config.clone();
        module_config.targets[index].import_name = Some(SWIFT_MODULE_PROBE.into());
        let oracle = Session::new(&inputs.entry, module_config.clone())?.generate()?;
        swift_identity(&oracle.files, SWIFT_MODULE_PROBE)?;
        last.session
            .as_mut()
            .context("last Session")?
            .set_config(module_config.clone())?;
        let (renamed, sample) = last.refresh(
            &inputs.entry,
            &module_config,
            &directory,
            &oracle.files,
            Stats {
                compiles: 0,
                renders: 1,
                cache_hits: config.targets.len() - 1,
            },
            None,
        )?;
        ensure!(
            Arc::ptr_eq(
                &renamed.contract,
                &last_baseline.as_ref().context("baseline")?.contract
            ),
            "Swift module configuration recompiled the contract"
        );
        ensure!(
            sample["removed_files"].as_u64().unwrap_or(0) > 0,
            "Swift module rename left obsolete sources"
        );
        last.session
            .as_mut()
            .context("last Session")?
            .set_config(config.clone())?;
        let (_, reverted) = last.refresh(
            &inputs.entry,
            &config,
            &directory,
            &baseline.files,
            hit,
            last_baseline.as_ref(),
        )?;
        json!({"change":sample,"revert":reverted,"package":SWIFT_PACKAGE,"default_module":SWIFT_PACKAGE,"explicit_module":SWIFT_MODULE_PROBE})
    } else {
        Value::Null
    };
    inputs.verify()?;
    ensure!(
        fingerprint(executable.to_string_lossy(), &fs::read(&executable)?) == binary,
        "running binary changed"
    );
    let configuration = json!({
        "targets":config.targets,"operation_ids":config.operation_ids,
        "cache_entries":config.cache_entries,"cache_bytes":config.cache_bytes,"owner":config.owner,
        "iterations":options.iterations,"warmups":options.warmups,
        "mode":if options.functional_only {"functional-only"} else {"observational"},
        "preparation":"private-canonical-json-v1","schedule":format!("{}-edit-revert-cycles-v1", options.schedule),
        "measurement":"system-allocator-generate-plus-owned-write-v1",
        "resource_attribution":if options.attribution {"posix-process-rusage-outside-interval-v1"} else {"disabled"},
    });
    let original_fingerprints = inputs.original_fingerprints()?;
    let report = json!({
        "format":FORMAT,"fixture":options.fixture,"generator_version":env!("CARGO_PKG_VERSION"),
        "build_profile":if cfg!(debug_assertions) {"debug"} else {"release"},
        "performance_status":if options.functional_only {"not-measured"} else {"observational"},"functional_status":"passed",
        "process":{"pid":std::process::id(),"started_at_ns":started_at_ns,"finished_at_ns":SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_nanos()},
        "binary":binary,"configuration_sha256":hash(serde_json::to_vec(&configuration)?),"configuration":configuration,
        "input_root":inputs.original_root,"private_root":inputs.private_root,
        "inputs":original_fingerprints,"input_sha256":hash(serde_json::to_vec(&original_fingerprints)?),
        "prepared_inputs":inputs.normalized,"prepared_input_sha256":hash(serde_json::to_vec(&inputs.normalized)?),
        "schema_nodes":baseline.contract.schemas().count(),
        "reference_edges":baseline.contract.schemas().map(|schema| schema.references().len()).sum::<usize>(),
        "selected_operations":operations.len(),
        "oracles":std::iter::once(json!({"scenario":"baseline","files":baseline.files.iter().map(|file| fingerprint(&file.path, file.content.as_bytes())).collect::<Vec<_>>()}))
            .chain(edits.iter().map(|edit| json!({"scenario":edit.name,"document":edit.source.document().as_path().and_then(|path| path.strip_prefix(&inputs.private_root).ok().map(Path::to_path_buf)),"pointer":edit.source.pointer(),"before_sha256":hash(&edit.before),"after_sha256":hash(&edit.after),"files":edit.expected.iter().map(|file| fingerprint(&file.path, file.content.as_bytes())).collect::<Vec<_>>()}))).collect::<Vec<_>>(),
        "samples":samples,"warmup_samples":warmup_samples,
        "configuration_probe":{"change":config_sample,"revert":config_revert},
        "module_probe":module_probe,
        "gates":{"zero_redundant_work":true,"zero_redundant_rewrites":true,"fresh_oracle_equal":true,
            "disk_current":true,"reverts_reuse_snapshots":true,"docs_only_executable_stable":true,
            "configuration_reuses_contract":true,"module_configuration_uses_target_cache":true,"original_inputs_unchanged":true,"private_edits_reverted":true},
        "measurement_scope":[
            "Cold creates a fresh Session; process, allocator, filesystem and OS page caches are not flushed.",
            "Refresh is generate (including cold Session::new) plus ownership comparison/staging/write. Source edits, copying, fresh oracles and checks are untimed.",
            "All inputs are private canonical JSON serialized from the copied closure; original and prepared bytes have separate hashes.",
            "Allocation calls and requested bytes count Rust System alloc/alloc_zeroed/realloc requests only, including realloc's full requested size. They are not live heap or C/tree-sitter allocations. Atomic instrumentation is enabled in all timing samples.",
            "rss_after_bytes is an untimed Linux VmRSS observation, not a per-refresh peak; unavailable hosts report null.",
            "Optional resources counters cover the current process and its threads, excluding children. getrusage is sampled just outside the timed/allocation interval, so its observation boundary is slightly wider. IO counts are OS block operations, not physical bytes; zero counts do not prove absence of IO. Process peak RSS is cumulative across the process lifetime, not a per-phase peak.",
            "position_in_cycle and ordinal record actual execution order; observed_at_elapsed_ms is recorded after untimed verification. Fixed and rotating schedules are distinct prospective workload identities and cannot be pooled as equivalent measurements.",
            "Warmups are preserved separately. Fixed-size recursive corpus observations do not prove asymptotic complexity.",
            "No package-manager/native consumer builds or downloads are included. Canonical native package gates measure those workloads.",
            "This executable enforces functional gates. Only the calibrated, compatible-runner comparator can establish a numerical gate."
        ],
    });
    fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}: {} checked refreshes, functional gates passed; performance {}; {}",
        options.fixture,
        samples.len(),
        if options.functional_only {
            "not measured"
        } else {
            "observational"
        },
        root.join("report.json").display()
    );
    Ok(())
}
