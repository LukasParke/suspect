//! Behavioral fragments: one focused OpenAPI document per cross-backend
//! behavior, executed through the *generator* of every backend that claims
//! the behavior.
//!
//! Where `tests/fragments.rs` pins admission verdicts, this suite pins
//! emission: each fragment states its general invariant in a
//! `# INVARIANT:` header, names the manifest feature it proves under
//! `x-suspect-feature`, and maps every covered backend to the exact
//! markers its generated SDK must carry (`x-suspect-behavior`). A backend
//! that stops emitting a marker fails here — before any native toolchain
//! runs, so the drift is caught hermetically.
//!
//! The coverage test ties the two manifests together: every
//! `(feature, backend)` claim in `src/features.rs` must be exercised by
//! at least one behavioral fragment, unless it is recorded in
//! `COVERAGE_DEBT` below. Debt entries the fragments have since covered
//! also fail, so the list can only shrink.

#![cfg(all(
    feature = "http-protocol",
    feature = "cpp-sdk",
    feature = "csharp-sdk",
    feature = "dart-sdk",
    feature = "java-sdk",
    feature = "kotlin-sdk",
    feature = "php-sdk",
    feature = "ruby-sdk"
))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig, generate_with_options};
use suspect_codegen::features::{BACKENDS, FEATURES, is_supported};
use suspect_codegen::sdk_defaults::SdkDefaults;
use suspect_ir::contract::Contract;
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};

fn behavioral_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/behavioral")
}

/// One parsed behavioral fragment.
struct Fragment {
    name: String,
    document: Value,
    feature: String,
    /// Whether emission is SDK-defaults-gated (the common case). When
    /// false, the no-policy control generation is skipped.
    gated: bool,
    /// Backend name → (file to scan, required markers).
    behaviors: BTreeMap<String, (String, Vec<String>)>,
}

/// Parses one fragment from YAML through the low/overlay stack.
fn load_fragment(name: &str) -> Fragment {
    let path = behavioral_dir().join(name);
    let text = std::fs::read_to_string(&path).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let low = LowDoc::parse(uri, Source::from_vec(text.as_bytes().to_vec()));
    let json = suspect_overlay::Value::from_node(low.root()).to_json();
    let document: Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("{name} must parse as a document: {e}"));

    let feature = document
        .get("x-suspect-feature")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{name} must carry `x-suspect-feature`"))
        .to_owned();
    assert!(
        FEATURES.iter().any(|f| f.id == feature),
        "{name}: unknown feature `{feature}`"
    );

    let gated = document
        .get("x-suspect-gated")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut behaviors = BTreeMap::new();
    let Some(map) = document
        .get("x-suspect-behavior")
        .and_then(Value::as_object)
    else {
        panic!("{name} must carry an `x-suspect-behavior` map");
    };
    for (backend, entry) in map {
        assert!(
            BACKENDS.contains(&backend.as_str()),
            "{name}: unknown backend `{backend}`"
        );
        let file = entry
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name}: `{backend}` must name a `file`"))
            .to_owned();
        let markers: Vec<String> = entry
            .get("markers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            !markers.is_empty(),
            "{name}: `{backend}` must list at least one marker"
        );
        behaviors.insert(backend.clone(), (file, markers));
    }
    assert!(!behaviors.is_empty(), "{name} covers no backends");

    Fragment {
        name: name.to_owned(),
        document,
        feature,
        gated,
        behaviors,
    }
}

/// Every behavioral fragment, sorted by file name.
fn all_fragments() -> Vec<Fragment> {
    let mut names: Vec<String> = std::fs::read_dir(behavioral_dir())
        .expect("behavioral directory")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yaml"))
        .collect();
    names.sort();
    names.iter().map(|n| load_fragment(n)).collect()
}

/// Per-backend generation identity, matching the conventions the
/// per-backend acceptance suites use.
fn target_for(backend: &str) -> TargetConfig {
    let (variant, package, version, import): (Backend, &str, &str, Option<&str>) = match backend {
        "cpp" => (Backend::CppHttp, "pagination_cpp", "0.1.0", None),
        "csharp" => (Backend::CsharpHttp, "acme.pagination-sdk", "0.1.0", None),
        "dart" => (Backend::DartHttp, "pagination_sdk", "0.1.0", None),
        "go" => (Backend::GoHttp, "example.com/pagination-sdk", "0.1.0", None),
        "java" => (
            Backend::JavaHttp,
            "test.suspect:pagination-java",
            "1.0.0",
            None,
        ),
        "kotlin" => (
            Backend::KotlinHttp,
            "test.suspect:pagination-kotlin",
            "0.1.0",
            None,
        ),
        "php" => (
            Backend::PhpHttp,
            "pagination/php-sdk",
            "1.0.0",
            Some("BehavioralFragment"),
        ),
        "python" => (
            Backend::PythonHttp,
            "pagination-sdk",
            "1.0.0",
            Some("pagination_sdk"),
        ),
        "ruby" => (Backend::RubyHttp, "pagination-sdk", "0.1.0", None),
        "rust" => (Backend::RustHttp, "pagination-rust", "0.1.0", None),
        "swift" => (Backend::SwiftHttp, "PaginationSDK", "0.1.0", None),
        "typescript" => (
            Backend::TypescriptHttp,
            "@behavioral/fixture",
            "0.0.0",
            None,
        ),
        other => panic!("no target config for `{other}`"),
    };
    TargetConfig {
        backend: variant,
        package_name: package.to_owned(),
        package_version: version.to_owned(),
        import_name: import.map(str::to_owned),
    }
}

/// Generates the fragment for one backend under SDK defaults.
fn generate_for(fragment: &Fragment, backend: &str) -> Vec<suspect_codegen::OutFile> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fragment.json");
    std::fs::write(&path, fragment.document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected: Vec<_> = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect();
    let options = GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    };
    generate_with_options(contract, &selected, &target_for(backend), &options).unwrap_or_else(
        |diagnostics| {
            panic!(
                "{}: generation for `{backend}` failed: {diagnostics:?}",
                fragment.name
            )
        },
    )
}

/// Generates the fragment for one backend without SDK defaults (control).
fn generate_control(fragment: &Fragment, backend: &str) -> Vec<suspect_codegen::OutFile> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fragment.json");
    std::fs::write(&path, fragment.document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected: Vec<_> = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect();
    generate_with_options(
        contract,
        &selected,
        &target_for(backend),
        &GenerationOptions::default(),
    )
    .unwrap()
}

/// Dumps every covered backend's configured generation to
/// `target/behavioral-dump/<fragment>/<backend>/…` for marker authoring.
/// Run with: `cargo test --test behavioral_fragments dump_generated_output -- --ignored --nocapture`
#[test]
#[ignore = "authoring helper: dumps generated SDK sources for marker writing"]
fn dump_generated_output() {
    for fragment in all_fragments() {
        for backend in fragment.behaviors.keys() {
            let files = generate_for(&fragment, backend);
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/behavioral-dump")
                .join(fragment.name.trim_end_matches(".yaml"))
                .join(backend);
            for file in files {
                let destination = root.join(&file.path);
                std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                std::fs::write(destination, file.content).unwrap();
            }
            eprintln!("dumped {backend} → {}", root.display());
        }
    }
}

#[test]
fn fragment_conventions_hold() {
    for fragment in all_fragments() {
        let text = std::fs::read_to_string(behavioral_dir().join(&fragment.name)).unwrap();
        assert!(
            text.contains("# INVARIANT:"),
            "{} must state its invariant in a `# INVARIANT:` comment",
            fragment.name
        );
        assert!(
            text.lines().count() >= 10,
            "{} is too small to be a focused document",
            fragment.name
        );
        // Every marker must be distinctive: at least 8 chars, no
        // whitespace-only entries.
        for (backend, (file, markers)) in &fragment.behaviors {
            assert!(
                !file.is_empty(),
                "{}: `{backend}` file is empty",
                fragment.name
            );
            for marker in markers {
                assert!(
                    marker.len() >= 8,
                    "{}: `{backend}` marker `{marker}` is too generic to pin behavior",
                    fragment.name
                );
            }
        }
    }
}

#[test]
fn markers_hold_under_sdk_defaults_and_stay_absent_without() {
    for fragment in all_fragments() {
        for (backend, (file, markers)) in &fragment.behaviors {
            let files = generate_for(&fragment, backend);
            let source = files
                .iter()
                .find(|f| f.path == *file)
                .unwrap_or_else(|| {
                    panic!(
                        "{}: `{backend}` did not emit `{file}`; emitted: {:?}",
                        fragment.name,
                        files.iter().map(|f| &f.path).collect::<Vec<_>>()
                    )
                })
                .content
                .clone();
            for marker in markers {
                assert!(
                    source.contains(marker.as_str()),
                    "{}: `{backend}` output `{file}` lacks marker `{marker}`\n--- emitted ---\n{source}",
                    fragment.name
                );
            }

            if fragment.gated {
                let control = generate_control(&fragment, backend);
                let bundle: String = control.iter().map(|f| f.content.clone()).collect();
                for marker in markers {
                    assert!(
                        !bundle.contains(marker.as_str()),
                        "{}: `{backend}` emits `{marker}` even without SDK defaults — the fragment cannot detect gating drift",
                        fragment.name
                    );
                }
            }
        }
    }
}

/// Feature claims with no behavioral-fragment coverage yet. A claim lands
/// here only when the per-backend native acceptance suite is the evidence;
/// entries must be removed as fragments grow, and a debt entry a fragment
/// already covers fails the suite.
const COVERAGE_DEBT: &[(&str, &str)] = &[
    // oauth2-client-credentials — exercised natively per backend (*_oauth.rs)
    ("oauth2-client-credentials", "cpp"),
    ("oauth2-client-credentials", "csharp"),
    ("oauth2-client-credentials", "dart"),
    ("oauth2-client-credentials", "go"),
    ("oauth2-client-credentials", "java"),
    ("oauth2-client-credentials", "kotlin"),
    ("oauth2-client-credentials", "php"),
    ("oauth2-client-credentials", "python"),
    ("oauth2-client-credentials", "ruby"),
    ("oauth2-client-credentials", "rust"),
    ("oauth2-client-credentials", "swift"),
    ("oauth2-client-credentials", "typescript"),
    // auth-replay-401 — same native suites
    ("auth-replay-401", "cpp"),
    ("auth-replay-401", "csharp"),
    ("auth-replay-401", "dart"),
    ("auth-replay-401", "go"),
    ("auth-replay-401", "java"),
    ("auth-replay-401", "kotlin"),
    ("auth-replay-401", "php"),
    ("auth-replay-401", "python"),
    ("auth-replay-401", "ruby"),
    ("auth-replay-401", "rust"),
    ("auth-replay-401", "swift"),
    ("auth-replay-401", "typescript"),
    // typed-stream-events — *_stream_typed.rs
    ("typed-stream-events", "cpp"),
    ("typed-stream-events", "csharp"),
    ("typed-stream-events", "dart"),
    ("typed-stream-events", "go"),
    ("typed-stream-events", "java"),
    ("typed-stream-events", "kotlin"),
    ("typed-stream-events", "php"),
    ("typed-stream-events", "python"),
    ("typed-stream-events", "ruby"),
    ("typed-stream-events", "rust"),
    ("typed-stream-events", "swift"),
    ("typed-stream-events", "typescript"),
    // incoming-webhooks — *_incoming.rs
    ("incoming-webhooks", "cpp"),
    ("incoming-webhooks", "csharp"),
    ("incoming-webhooks", "dart"),
    ("incoming-webhooks", "go"),
    ("incoming-webhooks", "java"),
    ("incoming-webhooks", "kotlin"),
    ("incoming-webhooks", "php"),
    ("incoming-webhooks", "python"),
    ("incoming-webhooks", "ruby"),
    ("incoming-webhooks", "rust"),
    ("incoming-webhooks", "swift"),
    ("incoming-webhooks", "typescript"),
    // env-var-credentials — *_credential_env.rs (csharp unclaimed entirely)
    ("env-var-credentials", "cpp"),
    ("env-var-credentials", "dart"),
    ("env-var-credentials", "go"),
    ("env-var-credentials", "java"),
    ("env-var-credentials", "kotlin"),
    ("env-var-credentials", "php"),
    ("env-var-credentials", "python"),
    ("env-var-credentials", "ruby"),
    ("env-var-credentials", "rust"),
    ("env-var-credentials", "swift"),
    ("env-var-credentials", "typescript"),
    // user-agent-attribution — *_attribution.rs (go unclaimed entirely);
    // python/rust/typescript are fragment-covered.
    ("user-agent-attribution", "cpp"),
    ("user-agent-attribution", "csharp"),
    ("user-agent-attribution", "dart"),
    ("user-agent-attribution", "java"),
    ("user-agent-attribution", "kotlin"),
    ("user-agent-attribution", "php"),
    ("user-agent-attribution", "ruby"),
    ("user-agent-attribution", "swift"),
];

#[test]
fn every_feature_claim_has_behavioral_fragment_coverage() {
    // Coverage as observed from the fragments.
    let mut covered: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for fragment in all_fragments() {
        let feature = FEATURES
            .iter()
            .find(|f| f.id == fragment.feature)
            .unwrap()
            .id;
        let entry = covered.entry(feature.to_owned()).or_default();
        for backend in fragment.behaviors.keys() {
            entry.insert(backend.clone());
        }
    }

    for (fi, feature) in FEATURES.iter().enumerate() {
        for (bi, backend) in BACKENDS.iter().enumerate() {
            if !is_supported(fi, bi) {
                continue;
            }
            let is_covered = covered
                .get(feature.id)
                .is_some_and(|set| set.contains(*backend));
            let is_debt = COVERAGE_DEBT
                .iter()
                .any(|(f, b)| *f == feature.id && *b == *backend);
            assert!(
                is_covered || is_debt,
                "{backend} claims `{}` but no behavioral fragment covers it and it is not in COVERAGE_DEBT — add markers to a fragment or record the debt explicitly",
                feature.id
            );
            assert!(
                !(is_covered && is_debt),
                "{backend} claim `{}` is covered by a fragment but still listed in COVERAGE_DEBT — remove the stale debt entry",
                feature.id
            );
        }
    }

    // Debt entries must reference real claims: a typo or an entry for an
    // unclaimed pair is dead weight.
    for (feature_id, backend) in COVERAGE_DEBT {
        let fi = FEATURES
            .iter()
            .position(|f| f.id == *feature_id)
            .unwrap_or_else(|| panic!("COVERAGE_DEBT names unknown feature `{feature_id}`"));
        let bi = BACKENDS
            .iter()
            .position(|b| b == backend)
            .unwrap_or_else(|| panic!("COVERAGE_DEBT names unknown backend `{backend}`"));
        assert!(
            is_supported(fi, bi),
            "COVERAGE_DEBT lists ({feature_id}, {backend}) but the manifest does not claim it"
        );
    }
}
