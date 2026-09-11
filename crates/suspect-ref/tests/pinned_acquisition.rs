//! Explicit acquisition/cache inputs exercised through the public Workspace seam.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use suspect_ref::acquire::{
    AcquireErrorKind, AcquireOptions, RefreshPolicy, acquire, parse_utc_timestamp,
};
use suspect_source::Uri;

mod pinned_support;
use pinned_support::{Server, options, pin, redirected_pin, response, tempdir, write_manifest};

const EMPTY_DIGEST: &str =
    "sha256-44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";

fn local_manifest(root: &Path) -> (PathBuf, Uri) {
    let path = root.join("entry.json");
    fs::write(&path, b"{}").unwrap();
    let entry = Uri::from_path(&path).unwrap();
    let manifest = root.join("pins.json");
    fs::write(
        &manifest,
        serde_json::to_vec_pretty(&json!({
            "manifest_version": 1,
            "entry": entry.as_str(),
            "resources": [{
                "requested_uri": entry.as_str(),
                "effective_uri": entry.as_str(),
                "digest": EMPTY_DIGEST,
                "media_type": "application/json",
                "via": "local",
                "redirects": [],
                "retrieved_at": "2026-09-09T00:00:00Z",
                "attempts": 0
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    (manifest, entry)
}

#[test]
fn missing_and_tampered_cache_entries_fail_closed_even_when_local_sources_exist() {
    for tamper in [false, true] {
        let root = tempdir();
        let (manifest, entry) = local_manifest(root.path());
        let mut options = AcquireOptions {
            cache_dir: root.path().join("cache"),
            ..AcquireOptions::default()
        };
        let snapshot = acquire(&manifest, options.clone()).unwrap();
        let path = snapshot.documents()[0].cache_path().unwrap();
        fs::remove_file(path).unwrap();
        if tamper {
            fs::write(path, b"[]").unwrap();
        }
        options.offline = true;
        let error = acquire(&manifest, options).unwrap_err();
        if tamper {
            assert!(matches!(
                error.kind(),
                AcquireErrorKind::DigestMismatch { .. }
            ));
        } else {
            assert_eq!(error.kind(), &AcquireErrorKind::CacheMiss);
        }
        assert_eq!(error.uri(), Some(&entry));
        assert_eq!(error.resource_index(), Some(0));
        assert_eq!(error.file_path(), Some(path));
        assert_eq!(snapshot.provider().document(&entry).unwrap().bytes(), b"{}");
    }
}

#[test]
fn cached_manifest_is_verified_and_no_half_cached_closure_is_returned() {
    let root = tempdir();
    let (manifest, entry) = local_manifest(root.path());
    let mut options = AcquireOptions {
        cache_dir: root.path().join("cache"),
        ..AcquireOptions::default()
    };
    let acquired = acquire(&manifest, options.clone()).unwrap();
    fs::remove_file(acquired.cache_manifest_path()).unwrap();
    options.offline = true;
    assert_eq!(
        acquire(&manifest, options.clone()).unwrap_err().kind(),
        &AcquireErrorKind::CacheMiss
    );
    fs::write(acquired.cache_manifest_path(), b"{}").unwrap();
    assert!(matches!(
        acquire(&manifest, options).unwrap_err().kind(),
        AcquireErrorKind::DigestMismatch { .. }
    ));
    let workspace = acquired.workspace_builder().build().unwrap();
    assert_eq!(
        workspace
            .open(entry.as_str())
            .unwrap()
            .doc()
            .inner()
            .bytes(),
        b"{}"
    );
}

#[test]
fn refresh_is_explicit_and_drift_does_not_rewrite_pins_or_replace_cached_bytes() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let root = tempdir();
    let changed = Arc::new(AtomicBool::new(false));
    let state = changed.clone();
    let server = Server::new(move |_| {
        response(
            200,
            &[("Content-Type", "application/json")],
            if state.load(Ordering::Acquire) {
                b"[]"
            } else {
                b"{}"
            },
        )
    });
    let entry = server.uri("/entry.json");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    let original = fs::read(&manifest).unwrap();
    let mut options = options(root.path(), &server);
    let first = acquire(&manifest, options.clone()).unwrap();
    changed.store(true, Ordering::Release);
    assert!(acquire(&manifest, options.clone()).unwrap().records()[0].from_cache());
    options.refresh = RefreshPolicy::StaleOnly {
        before: parse_utc_timestamp("2026-09-08T00:00:00Z").unwrap(),
    };
    assert!(acquire(&manifest, options.clone()).unwrap().records()[0].from_cache());
    assert_eq!(server.requests().len(), 1);
    options.refresh = RefreshPolicy::StaleOnly {
        before: parse_utc_timestamp("2026-09-10T00:00:00Z").unwrap(),
    };
    assert!(matches!(
        acquire(&manifest, options.clone()).unwrap_err().kind(),
        AcquireErrorKind::DigestDrift { .. }
    ));
    options.refresh = RefreshPolicy::All;
    assert!(matches!(
        acquire(&manifest, options.clone()).unwrap_err().kind(),
        AcquireErrorKind::DigestDrift { .. }
    ));
    assert_eq!(
        server.requests().len(),
        3,
        "one attempt per explicit refresh"
    );
    assert_eq!(fs::read(&manifest).unwrap(), original);
    assert_eq!(
        fs::read(first.documents()[0].cache_path().unwrap()).unwrap(),
        b"{}"
    );
    options.offline = true;
    options.refresh = RefreshPolicy::Never;
    let offline = acquire(&manifest, options).unwrap();
    assert_eq!(offline.provider().document(&entry).unwrap().bytes(), b"{}");
}

#[test]
fn a_refresh_never_silently_repairs_a_tampered_cache_entry() {
    let root = tempdir();
    let server = Server::new(|_| response(200, &[("Content-Type", "application/json")], b"{}"));
    let entry = server.uri("/entry.json");
    let manifest = write_manifest(root.path(), &entry, vec![pin(&entry, b"{}")]);
    let mut options = options(root.path(), &server);
    let acquired = acquire(&manifest, options.clone()).unwrap();
    let path = acquired.documents()[0].cache_path().unwrap();
    fs::remove_file(path).unwrap();
    fs::write(path, b"[]").unwrap();
    options.refresh = RefreshPolicy::All;
    assert!(matches!(
        acquire(&manifest, options).unwrap_err().kind(),
        AcquireErrorKind::DigestMismatch { .. }
    ));
    assert_eq!(
        server.requests().len(),
        1,
        "tamper is detected before refresh network I/O"
    );
}

#[test]
fn redirected_remote_entry_uses_effective_identity_through_offline_contract_compilation() {
    use std::sync::Arc;
    use suspect_ir::contract::Contract;
    let root = tempdir();
    let entry_bytes = br##"{"openapi":"3.1.0","info":{"title":"Redirect","version":"1"},"paths":{},"components":{"schemas":{"Use":{"$ref":"schemas.json#/Pet"}}}}"##;
    let schema = br#"{"Pet":{"type":"string"}}"#;
    let server = Server::new(move |request| match request.path.as_str() {
        "/latest.json" => response(302, &[("Location", "/release")], b"unused"),
        "/release" => response(307, &[("Location", "/v2/openapi.json")], b"unused"),
        "/v2/openapi.json" => response(200, &[("Content-Type", "application/json")], entry_bytes),
        "/v2/schemas.json" => response(200, &[("Content-Type", "application/json")], schema),
        other => panic!("unexpected retrieval {other}"),
    });
    let requested = server.uri("/latest.json");
    let middle = server.uri("/release");
    let effective = server.uri("/v2/openapi.json");
    let leaf = server.uri("/v2/schemas.json");
    let manifest = write_manifest(
        root.path(),
        &requested,
        vec![
            redirected_pin(
                &requested,
                &effective,
                entry_bytes,
                &[(&requested, &middle, 302), (&middle, &effective, 307)],
            ),
            pin(&leaf, schema),
        ],
    );
    let mut options = options(root.path(), &server);
    let acquired = acquire(&manifest, options.clone()).unwrap();
    assert_eq!(acquired.entry(), &effective);
    assert_eq!(acquired.requested_entry(), &requested);
    assert_eq!(acquired.records()[0].attempts(), 3);
    assert_eq!(acquired.records()[0].redirects().len(), 2);
    assert!(
        !acquired.logical_uris().contains(&middle),
        "intermediate hops are not document lookup aliases"
    );
    drop(server);
    options.offline = true;
    options.insecure_test_origins.clear();
    let offline = acquire(&manifest, options).unwrap();
    let workspace = Arc::new(offline.workspace_builder().build().unwrap());
    let contract = Contract::from_workspace(&workspace, offline.entry()).unwrap();
    assert!(
        contract.diagnostics().is_empty(),
        "{:?}",
        contract.diagnostics()
    );
    assert_eq!(contract.entry(), &effective);
    assert_eq!(contract.documents().count(), 2);
    assert_eq!(contract.document(&leaf).unwrap()["Pet"]["type"], "string");
    assert_eq!(
        workspace.open(requested.as_str()).unwrap().id(),
        workspace.open(effective.as_str()).unwrap().id()
    );
    assert_eq!(workspace.len(), 2);
}

#[test]
fn all_manifest_pins_are_validated_before_any_retrieval() {
    let root = tempdir();
    let server = Server::new(|_| panic!("invalid manifest must not issue any request"));
    let entry = server.uri("/entry.json");
    let base = pin(&entry, b"{}");
    let mut conflicts = base.clone();
    conflicts["digest"] = json!(suspect_ref::sha256_digest(b"[]"));
    let manifest = write_manifest(root.path(), &entry, vec![base.clone(), conflicts]);
    assert_eq!(
        acquire(&manifest, options(root.path(), &server))
            .unwrap_err()
            .kind(),
        &AcquireErrorKind::ConflictingIdentity
    );
    let other = server.uri("/other.json");
    write_manifest(root.path(), &entry, vec![pin(&other, b"{}")]);
    assert!(matches!(
        acquire(&manifest, options(root.path(), &server))
            .unwrap_err()
            .kind(),
        AcquireErrorKind::InvalidManifest { .. }
    ));
    for (field, value) in [
        ("digest", json!("sha256-wrong")),
        (
            "requested_uri",
            json!("https://user:do-not-leak@spec.example.test/file.json"),
        ),
        (
            "effective_uri",
            json!("https://spec.example.test/file.json#fragment"),
        ),
        ("retrieved_at", json!("2026-02-30T00:00:00Z")),
        ("attempts", json!(2)),
        ("media_type", json!("text/html")),
        ("undeclared_secret", json!("do-not-leak")),
    ] {
        let mut resource = base.clone();
        resource[field] = value;
        write_manifest(root.path(), &entry, vec![resource]);
        let error = acquire(&manifest, options(root.path(), &server)).unwrap_err();
        assert!(!format!("{error} {error:?}").contains("do-not-leak"));
    }
    assert!(server.requests().is_empty());
}

#[test]
fn closure_count_manifest_size_and_aggregate_byte_caps_apply_to_acquisition_and_offline_verification()
 {
    let root = tempdir();
    let server = Server::new(|_| response(200, &[("Content-Type", "application/json")], b"{}"));
    let entry = server.uri("/a.json");
    let second = server.uri("/b.json");
    let manifest = write_manifest(
        root.path(),
        &entry,
        vec![pin(&entry, b"{}"), pin(&second, b"{}")],
    );
    let mut config = options(root.path(), &server);
    config.max_docs = 1;
    assert!(matches!(
        acquire(&manifest, config.clone()).unwrap_err().kind(),
        AcquireErrorKind::TooManyDocuments { limit: 1 }
    ));
    assert!(server.requests().is_empty());
    config.max_docs = 2;
    config.max_manifest_bytes = 2;
    assert!(matches!(
        acquire(&manifest, config.clone()).unwrap_err().kind(),
        AcquireErrorKind::ManifestTooLarge { limit: 2 }
    ));
    assert!(server.requests().is_empty());
    config.max_manifest_bytes = 4096;
    config.max_total_bytes = 3;
    assert!(matches!(
        acquire(&manifest, config.clone()).unwrap_err().kind(),
        AcquireErrorKind::TotalTooLarge { limit: 3 }
    ));
    assert!(
        !config.cache_dir.exists(),
        "a failed acquisition does not publish a partial cache"
    );
    config.max_total_bytes = 4;
    let acquired = acquire(&manifest, config.clone()).unwrap();
    assert_eq!(acquired.documents().len(), 2);
    config.offline = true;
    config.max_total_bytes = 3;
    assert!(matches!(
        acquire(&manifest, config).unwrap_err().kind(),
        AcquireErrorKind::TotalTooLarge { limit: 3 }
    ));
}

#[test]
fn local_entry_is_pinned_and_offline_provider_is_an_immutable_byte_snapshot() {
    let root = tempdir();
    let (manifest, entry) = local_manifest(root.path());
    let cache_dir = root.path().join("cache");
    let acquired = acquire(
        &manifest,
        AcquireOptions {
            cache_dir: cache_dir.clone(),
            ..AcquireOptions::default()
        },
    )
    .unwrap();
    assert_eq!(acquired.documents()[0].digest(), EMPTY_DIGEST);
    fs::remove_file(entry.as_path().unwrap()).unwrap();
    let offline = acquire(
        &manifest,
        AcquireOptions {
            cache_dir,
            offline: true,
            ..AcquireOptions::default()
        },
    )
    .unwrap();
    assert_eq!(offline.entry(), &entry);
    assert_eq!(offline.fingerprint(), acquired.fingerprint());
    fs::remove_file(offline.documents()[0].cache_path().unwrap()).unwrap();
    let workspace = offline.workspace_builder().build().unwrap();
    let document = workspace.open(offline.entry().as_str()).unwrap();
    assert_eq!(document.doc().inner().bytes(), b"{}");
    assert_eq!(
        document.uri(),
        &entry,
        "cache paths never become source identity"
    );
}

#[test]
fn loopback_acquisition_then_offline_contract_resolves_remote_relative_references() {
    use std::sync::Arc;
    use suspect_ir::contract::{Contract, SourceId};
    use suspect_low::Pointer;

    let root = tempdir();
    let leaf = br#"{"Name":{"type":"string"}}"#;
    // A root $id intentionally changes scoped JSON Schema reference bases.
    // Keep the identifier in instance data so it cannot become a retrieval.
    let remote = br##"{"Value":{"type":"object","properties":{"name":{"$ref":"leaf.json#/Name"}}},"example":{"$id":"https://identifiers.example.test/not-a-download"}}"##;
    let server = Server::new(move |request| match request.path.as_str() {
        "/schemas/root.json" => response(
            200,
            &[("Content-Type", "application/json; charset=utf-8")],
            remote,
        ),
        "/schemas/leaf.json" => response(200, &[("Content-Type", "application/json")], leaf),
        other => panic!("undeclared acquisition: {other}"),
    });
    let remote_uri = server.uri("/schemas/root.json");
    let leaf_uri = server.uri("/schemas/leaf.json");
    let entry_path = root.path().join("openapi.json");
    let entry = Uri::from_path(&entry_path).unwrap();
    let entry_bytes = serde_json::to_vec(&json!({
        "openapi": "3.1.0", "info": {"title": "Pins", "version": "1.0.0"}, "paths": {},
        "components": {"schemas": {"Value": {"$ref": format!("{remote_uri}#/Value")}}}
    }))
    .unwrap();
    fs::write(&entry_path, &entry_bytes).unwrap();
    let manifest = write_manifest(
        root.path(),
        &entry,
        vec![
            pin(&entry, &entry_bytes),
            pin(&remote_uri, remote),
            pin(&leaf_uri, leaf),
        ],
    );
    let cache_dir = root.path().join("cache");
    let acquired = acquire(
        &manifest,
        AcquireOptions {
            cache_dir: cache_dir.clone(),
            insecure_test_origins: vec![server.origin.clone()],
            ..AcquireOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>(),
        vec!["/schemas/root.json", "/schemas/leaf.json"]
    );
    assert_eq!(acquired.documents().len(), 3);
    drop(server);
    fs::remove_file(entry_path).unwrap();
    let offline = acquire(
        &manifest,
        AcquireOptions {
            cache_dir,
            offline: true,
            ..AcquireOptions::default()
        },
    )
    .unwrap();
    let workspace = Arc::new(offline.workspace_builder().build().unwrap());
    let contract = Contract::from_workspace(&workspace, offline.entry()).unwrap();
    assert!(
        contract.diagnostics().is_empty(),
        "{:?}",
        contract.diagnostics()
    );
    assert_eq!(contract.documents().count(), 3);
    let source = SourceId::new(leaf_uri.clone(), Pointer::parse("/Name").unwrap());
    assert_eq!(contract.source(&source).unwrap()["type"], "string");
    assert!(contract.schema(&source).is_some());
    assert_eq!(workspace.get(&leaf_uri).unwrap().uri(), &leaf_uri);
}
