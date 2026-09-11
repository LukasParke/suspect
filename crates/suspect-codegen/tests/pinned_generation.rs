//! Pinned source bytes, rather than cache paths or live originals, drive sessions.

use std::{path::PathBuf, sync::Arc};

use serde_json::json;
use sha2::{Digest, Sha256};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    generation_session::{Input, Session, SessionConfig, SessionError},
};
use suspect_ref::acquire::{AcquireErrorKind, AcquireOptions, acquire};
use suspect_source::Uri;

struct Fixture {
    root: tempfile::TempDir,
    input: Input,
    source_uri: Uri,
    cached_model: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let entry = root.path().join("api.json");
        let model = root.path().join("model.json");
        let source_uri = Uri::from_path(&entry).unwrap();
        let model_uri = Uri::from_path(&model).unwrap();
        let model_bytes=br#"{"type":"object","required":["name"],"properties":{"name":{"type":"string"}},"additionalProperties":false}"#;
        let source = json!({"openapi":"3.1.0","info":{"title":"Pinned","version":"1"},
            "servers":[{"url":"https://api.example.test/v1"}],"security":[{"Bearer":[]}],
            "components":{"securitySchemes":{"Bearer":{"type":"http","scheme":"bearer"}}},
            "paths":{"/things":{"get":{"operationId":"getThings","responses":{"200":{"description":"Things",
                "content":{"application/json":{"schema":{"$ref":"./model.json"}}}}}}}}});
        let entry_bytes = serde_json::to_vec(&source).unwrap();
        std::fs::write(&entry, &entry_bytes).unwrap();
        std::fs::write(&model, model_bytes).unwrap();
        let pin = |uri: &Uri, bytes: &[u8]| {
            json!({"requested_uri":uri.as_str(),"effective_uri":uri.as_str(),
            "digest":format!("sha256-{:x}",Sha256::digest(bytes)),"media_type":"application/json","via":"local",
            "redirects":[],"retrieved_at":"2026-09-10T00:00:00Z","attempts":0})
        };
        let manifest = root.path().join("pins.json");
        let cache_dir = root.path().join("cache");
        std::fs::write(
            &manifest,
            serde_json::to_vec(&json!({"manifest_version":1,"entry":source_uri.as_str(),
            "resources":[pin(&source_uri,&entry_bytes),pin(&model_uri,model_bytes)]}))
            .unwrap(),
        )
        .unwrap();
        let pins = acquire(
            &manifest,
            AcquireOptions {
                cache_dir: cache_dir.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        let cached_model = pins
            .documents()
            .iter()
            .find(|doc| doc.effective_uri() == &model_uri)
            .unwrap()
            .cache_path()
            .unwrap()
            .to_owned();
        std::fs::remove_file(entry).unwrap();
        std::fs::remove_file(model).unwrap();
        Self {
            root,
            input: Input::Pinned {
                manifest,
                cache_dir,
                insecure_test_origins: Vec::new(),
            },
            source_uri,
            cached_model,
        }
    }

    fn session(&self) -> Session {
        Session::with_input(
            self.input.clone(),
            SessionConfig {
                targets: vec![TargetConfig {
                    backend: Backend::TypescriptHttp,
                    package_name: "pinned-sdk".into(),
                    package_version: "1.0.0".into(),
                    import_name: None,
                }],
                ..Default::default()
            },
        )
        .unwrap()
    }
}

#[test]
fn offline_generation_reuses_pinned_contract_and_keeps_logical_source_identity() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    let cold = session.generate().unwrap();
    assert_eq!(cold.contract.entry(), &fixture.source_uri);
    assert_eq!((cold.delta.compiles, cold.delta.renders), (1, 1));
    assert!(cold.new_documents.contains(&fixture.source_uri.to_string()));
    let forbidden = fixture.root.path().join("cache").display().to_string();
    assert!(
        cold.files
            .iter()
            .all(|file| !file.content.contains(&forbidden))
    );
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    assert_eq!(cold.revision, warm.revision);
    assert!(Arc::ptr_eq(&cold.contract, &warm.contract));
    assert!(Arc::ptr_eq(&cold.files, &warm.files));
}

#[test]
fn warm_pinned_session_detects_cache_tampering_before_reusing_artifacts() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    session.generate().unwrap();
    let mut permissions = std::fs::metadata(&fixture.cached_model)
        .unwrap()
        .permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o600);
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    std::fs::set_permissions(&fixture.cached_model, permissions).unwrap();
    std::fs::write(&fixture.cached_model, b"{}").unwrap();
    let error = session.generate().unwrap_err();
    assert!(
        matches!(error,SessionError::Acquisition(error) if matches!(error.kind(),AcquireErrorKind::DigestMismatch{..}))
    );
    assert_eq!(session.stats().compiles, 1);
}

#[test]
fn warm_pinned_session_never_ignores_a_missing_cached_document() {
    let fixture = Fixture::new();
    let mut session = fixture.session();
    session.generate().unwrap();
    std::fs::remove_file(&fixture.cached_model).unwrap();
    let error = session.generate().unwrap_err();
    assert!(
        matches!(error,SessionError::Acquisition(error) if matches!(error.kind(),AcquireErrorKind::CacheMiss))
    );
}
