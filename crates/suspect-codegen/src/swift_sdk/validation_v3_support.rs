//! Test-only closed source loading and retained native-tool evidence for Swift v3.
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

pub use super::validation_v2_support::{checked, compiler, directory, docs, swift};

pub const ENTRY: &str = "https://swift-resources.test/api.json";

pub fn root(label: &str) -> PathBuf {
    let base = std::env::var_os("SUSPECT_SWIFT_V3_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-v3-gates"));
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}

pub fn source(document: &str, pointer: &str) -> SchemaId {
    pointer.split('/').skip(1).fold(
        SchemaId::new(Uri::parse(document).unwrap(), Default::default()),
        |source, token| source.child(&token.replace("~1", "/").replace("~0", "~")),
    )
}

pub fn schema(name: &str) -> SchemaId {
    source(ENTRY, "/components/schemas").child(name)
}

pub fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Swift resource witnesses","version":"1"},"paths":{},"components":{"schemas":schemas}})
}

pub fn provided(entry: &str, documents: Vec<(&str, &str, Vec<u8>)>) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(requested, effective, bytes)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                bytes,
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap())
}

pub fn load(document: Value, external: Vec<(&str, Value)>) -> Arc<Contract> {
    let mut documents = vec![(ENTRY, ENTRY, document.to_string().into_bytes())];
    documents.extend(
        external
            .into_iter()
            .map(|(uri, value)| (uri, uri, value.to_string().into_bytes())),
    );
    provided(ENTRY, documents)
}
