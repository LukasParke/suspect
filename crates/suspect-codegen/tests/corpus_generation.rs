//! Generation stress gate over real-world OpenAPI descriptions.
//!
//! Generates every backend's SDK for every operation of every document in
//! `corpus/` (`cargo run -p xtask -- fetch-corpus`). Refusals are expected
//! and fine (ADR 0010); what must never happen is an internal generator
//! error — a panic caught at the `backend::generate` boundary — on any
//! input. The per-backend refusal summary is written to
//! `target/corpus-generation.json` for coverage review.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_codegen::backend::{self, Backend, TargetConfig};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

/// Known-valid package identities per backend (mirrors xtask's sdk-full).
fn target(backend: Backend) -> TargetConfig {
    let (package, import): (&str, Option<&str>) = match backend.name() {
        "typescript-http" => ("@suspect-fixtures/corpus", None),
        "rust-http" => ("corpus-sdk", None),
        "python-http" => ("corpus-sdk", Some("corpus_sdk")),
        "go-http" => ("example.com/corpus-sdk", None),
        "swift-http" => ("CorpusSdk", Some("CorpusSdk")),
        "java-http" => ("com.example.corpus:corpus-sdk", Some("com.example.corpus")),
        "csharp-http" => ("Suspect.CorpusSdk", Some("Suspect.CorpusSdk")),
        "kotlin-http" => ("com.example:corpus-sdk", Some("example.corpus")),
        "ruby-http" => ("corpus-sdk", Some("CorpusSdk")),
        "php-http" => ("example/corpus-sdk", Some("Example\\CorpusSdk")),
        "dart-http" => ("corpus_sdk", None),
        "cpp-http" => ("corpus_sdk", Some("corpus_sdk")),
        other => panic!("no package identity for backend {other}"),
    };
    TargetConfig {
        backend,
        package_name: package.into(),
        package_version: "1.0.0".into(),
        import_name: import.map(str::to_owned),
    }
}

fn corpus() -> Option<Vec<PathBuf>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yaml" || ext == "json")
        })
        .collect();
    files.sort();
    (!files.is_empty()).then_some(files)
}

fn contract(path: &Path) -> Result<Arc<Contract>, String> {
    let root = path.parent().ok_or("no parent")?;
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(root)
            .build()
            .map_err(|e| e.to_string())?,
    );
    let uri = Uri::from_path(path).map_err(|e| e.to_string())?;
    Contract::from_workspace(&workspace, &uri)
        .map(Arc::new)
        .map_err(|e| e.to_string())
}

#[test]
#[ignore = "requires `cargo run -p xtask -- fetch-corpus`"]
fn no_backend_has_an_internal_error_on_real_world_descriptions() {
    let Some(files) = corpus() else {
        eprintln!("skipping: corpus/ is empty; run `cargo run -p xtask -- fetch-corpus`");
        return;
    };
    let mut summary: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
    let mut internal = Vec::new();
    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = files
            .iter()
            .map(|file| {
                scope.spawn(move || {
                    let name = file.file_name().unwrap().to_string_lossy().into_owned();
                    let contract = match contract(file) {
                        Ok(contract) => contract,
                        Err(error) => return (name, Err(error)),
                    };
                    let operations: Vec<_> = contract
                        .operations()
                        .map(|op| op.source().clone())
                        .collect();
                    let per_backend: Vec<_> = Backend::ALL
                        .iter()
                        .map(|backend| {
                            let outcome =
                                backend::generate(contract.clone(), &operations, &target(*backend));
                            (backend.name(), operations.len(), outcome)
                        })
                        .collect();
                    (name, Ok(per_backend))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread"))
            .collect::<Vec<_>>()
    });
    for (name, outcome) in results {
        let entry = summary.entry(name.clone()).or_default();
        match outcome {
            Err(error) => {
                entry.insert("contract".into(), serde_json::json!({ "error": error }));
            }
            Ok(per_backend) => {
                for (backend, operations, outcome) in per_backend {
                    let value = match outcome {
                        Ok(files) => {
                            serde_json::json!({ "operations": operations, "files": files.len() })
                        }
                        Err(diagnostics) => {
                            let mut codes: BTreeMap<&str, usize> = BTreeMap::new();
                            for diagnostic in &diagnostics {
                                *codes.entry(diagnostic.code).or_default() += 1;
                                if diagnostic.code == "sdk-internal-error" {
                                    internal.push(format!(
                                        "{name} / {backend}: {}",
                                        diagnostic.message
                                    ));
                                }
                            }
                            serde_json::json!({ "operations": operations, "refused": codes })
                        }
                    };
                    entry.insert(backend.into(), value);
                }
            }
        }
    }
    let report = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/corpus-generation.json");
    std::fs::write(&report, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    assert!(
        internal.is_empty(),
        "internal generator errors ({}), report at {}:\n{}",
        internal.len(),
        report.display(),
        internal.join("\n")
    );
}
