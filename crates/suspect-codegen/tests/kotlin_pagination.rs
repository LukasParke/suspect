//! M3 pagination runtime emission for the Kotlin backend: generation-time
//! emission shape, the no-policy control, and native compilation of the
//! emitted package when a JDK/Maven toolchain is available. Static runtime
//! files are never modified; every walker lives in the emitted `Client.kt`.

#![cfg(feature = "kotlin-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi":"3.1.0",
        "info":{"title":"Pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths":{
            "/widgets":{"get":{
                "operationId":"listWidgets",
                "parameters":[
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}},
                    {"name":"offset","in":"query","schema":{"type":"integer","minimum":0}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "data":{"type":"array","items":{"$ref":"#/components/schemas/Widget"}},
                        "total":{"type":"integer"}
                    }}}}}}}
            },
            "/events":{"get":{
                "operationId":"listEvents",
                "parameters":[
                    {"name":"cursor","in":"query","schema":{"type":"string"}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "items":{"type":"array","items":{"type":"string"}},
                        "next_page_token":{"type":"string"}
                    }}}}}}}
            }
        },
        "components":{"schemas":{"Widget":{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}}}
    })
}

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document()).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::KotlinHttp,
        package_name: "test.suspect:pagination-kotlin".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    generate_document(document(), options)
}

fn generate_document(document: Value, options: &GenerationOptions) -> Vec<OutFile> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract = Arc::new(Contract::from_workspace(&workspace, &entry).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
        ..Default::default()
    }
}

fn client_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path.ends_with("pagination_kotlin/Client.kt"))
        .expect("generated Client.kt")
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

#[test]
fn configured_emission_adds_no_files_and_the_client_gains_the_walkers() {
    let mut configured = generate(&configured_options());
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut configured);
    sorted(&mut control);
    // Emission adds no package files at all: only Client.kt changes.
    assert_eq!(
        configured
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
        control
            .iter()
            .map(|file| &file.path)
            .collect::<std::collections::BTreeSet<_>>(),
        "pagination emission must not add or remove package files"
    );
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if file.path.ends_with("pagination_kotlin/Client.kt") {
            assert_ne!(
                emitted.content, file.content,
                "the paginated client gains the walkers"
            );
        } else {
            assert_eq!(emitted.content, file.content, "{} changed", file.path);
        }
    }
    let client = client_file(&configured).content.clone();
    for expected in [
        // The typed traversal exception, emitted exactly once.
        "public class PaginationStalled(",
        // Limit/offset walk: cold page flow, item flow, and the next-input
        // builder with the documented re-collection semantics.
        "public fun listWidgetsPages(input: ListWidgetsInput = ListWidgetsInput(), requestOptions: RequestOptions = RequestOptions()): Flow<ListWidgetsResult> = flow {",
        "public fun listWidgetsItems(input: ListWidgetsInput = ListWidgetsInput(), requestOptions: RequestOptions = RequestOptions()): Flow<Widget>",
        "public suspend fun listWidgetsNextPage(input: ListWidgetsInput = ListWidgetsInput(), requestOptions: RequestOptions = RequestOptions()): ListWidgetsInput?",
        "for (item in listWidgetsPageItems(page.data) ?: emptyList()) emit(item)",
        // The private typed accessors over the decoded page model.
        "private fun listWidgetsPageItems(page: ListWidgetsResponse200): List<Widget>?",
        // Continuation mechanics: initial offset fill, exact advance, rebuild.
        "if (current.offset is Presence.Absent) current = current.copy(offset = Presence.Present(JsonNumber.of(0L)))",
        "base.toBigIntegerExact().add(java.math.BigInteger.valueOf(count.toLong()))",
        "return current.copy(offset = Presence.Present(JsonNumber.of(advanced)))",
        "if (count == 0) return null",
        // Cursor walk: pointer-driven continuation with the repeat guard.
        "public fun listEventsPages(input: ListEventsInput = ListEventsInput(), requestOptions: RequestOptions = RequestOptions()): Flow<ListEventsResult>",
        "public fun listEventsItems(input: ListEventsInput = ListEventsInput(), requestOptions: RequestOptions = RequestOptions()): Flow<String>",
        "public suspend fun listEventsNextPage(input: ListEventsInput = ListEventsInput(), requestOptions: RequestOptions = RequestOptions()): ListEventsInput?",
        "private fun listEventsPageContinuation(page: ListEventsResponse200): String?",
        "if (token.isEmpty()) return null",
        "throw PaginationStalled(token, \"listEvents\",",
        "return current.copy(cursor = Presence.Present(token))",
    ] {
        assert!(
            client.contains(expected),
            "Client.kt lacks:\n{expected}\n--- emitted: ---\n{client}"
        );
    }
    let control_client = client_file(&control).content.clone();
    for absent in [
        "listWidgetsPages",
        "listWidgetsItems",
        "listWidgetsNextPage",
        "PaginationStalled",
        "listEventsPageContinuation",
    ] {
        assert!(
            !control_client.contains(absent),
            "no-policy Client.kt gained {absent}"
        );
    }
}

#[test]
fn policy_without_paginated_operations_leaves_everything_byte_identical() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({"version":"v1","pagination":"off"})).unwrap(),
        ),
        ..Default::default()
    };
    let mut disabled = generate(&off);
    let mut control = generate(&GenerationOptions::default());
    sorted(&mut disabled);
    sorted(&mut control);
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[test]
fn plan_carries_the_compiled_selection_only_when_configured() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = suspect_codegen::kotlin_sdk::plan_sdk(
        contract.clone(),
        &selected,
        suspect_codegen::kotlin_sdk::SdkConfig {
            group_id: "test.suspect".into(),
            artifact_id: "pagination-kotlin".into(),
            version: "0.1.0".into(),
            package_name: "test.suspect.pagination_kotlin".into(),
            sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    let pagination = configured
        .pagination()
        .expect("configured policy is carried");
    assert!(pagination.emits());
    assert_eq!(pagination.outcome.paginated.len(), 2);
    assert_eq!(pagination.operations.len(), 2);
    let widgets = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "listWidgets")
        .expect("limit/offset operation");
    assert_eq!(
        widgets.advance,
        suspect_codegen::kotlin_sdk::Advance::OffsetItemCount
    );
    assert_eq!(widgets.initial_offset, Some(0));
    assert_eq!(widgets.control.name, "offset");
    assert_eq!(
        widgets.control.kind,
        suspect_codegen::kotlin_sdk::ControlKind::Integer
    );
    assert!(widgets.control.optional);
    assert_eq!(widgets.pages, "listWidgetsPages");
    assert_eq!(widgets.items.as_deref(), Some("listWidgetsItems"));
    assert_eq!(widgets.next_page, "listWidgetsNextPage");
    assert_eq!(widgets.data_type.as_deref(), Some("ListWidgetsResponse200"));
    let events = pagination
        .operations
        .iter()
        .find(|operation| operation.method == "listEvents")
        .expect("cursor operation");
    assert_eq!(
        events.advance,
        suspect_codegen::kotlin_sdk::Advance::CursorPointer
    );
    assert_eq!(events.control.name, "cursor");
    assert_eq!(
        events.control.kind,
        suspect_codegen::kotlin_sdk::ControlKind::Text
    );
    assert_eq!(events.pages, "listEventsPages");
    let control =
        suspect_codegen::kotlin_sdk::plan_sdk(contract, &selected, Default::default()).unwrap();
    assert!(control.pagination().is_none());
}

/// A second contract exercising every remaining walker template through
/// manual mappings: page-number walks with has-more, a top-level array page,
/// a nested next-offset pointer, and a required cursor parameter.
fn manual_document() -> Value {
    let rows = json!({
        "get": {
            "operationId": "listRows",
            "parameters": [
                {"name":"perPage","in":"query","schema":{"type":"integer","minimum":1}},
                {"name":"page","in":"query","schema":{"type":"integer","minimum":1}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["records","paging"],"properties":{
                        "records":{"type":"array","items":{"type":"integer"}},
                        "paging":{"type":"object","required":["more"],"properties":{
                            "more":{"type":"boolean"},
                            "total":{"type":"integer"}}
                        }
                    }}}}}
            }
        }
    });
    let blobs = json!({
        "get": {
            "operationId": "listBlobs",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"array","items":{"type":"string"}}}}}
            }
        }
    });
    let alerts = json!({
        "get": {
            "operationId": "listAlerts",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["entries","paging"],"properties":{
                        "entries":{"type":"array","items":{"type":"string"}},
                        "paging":{"type":"object","required":["count"],"properties":{
                            "count":{"type":"integer"}}
                        }
                    }}}}}
            }
        }
    });
    let audit = json!({
        "get": {
            "operationId": "listAuditEvents",
            "parameters": [
                {"name":"after","in":"query","required":true,"schema":{"type":"string"}}
            ],
            "responses": {
                "200": {"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","required":["events","next"],"properties":{
                        "events":{"type":"array","items":{"type":"string"}},
                        "next":{"type":"string"}
                    }}}}}
            }
        }
    });
    json!({
        "openapi":"3.1.0",
        "info":{"title":"Manual pagination","version":"1"},
        "servers":[{"url":"https://api.pagination.test/v1"}],
        "paths": {"/rows": rows, "/blobs": blobs, "/alerts": alerts, "/audit": audit}
    })
}

fn manual_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1",
                "pagination": {"mode":"auto","operations":{
                    "listRows": {
                        "pattern":"page-number",
                        "request":{"page":"page","limit":"perPage"},
                        "response":{"items":"/records","has-more":"/paging/more","total":"/paging/total"}
                    },
                    "listBlobs": {
                        "pattern":"limit-offset",
                        "request":{"limit":"limit","offset":"offset"},
                        "response":{"items":""},
                        "initial_offset":0,
                        "advance":"items-returned"
                    },
                    "listAlerts": {
                        "pattern":"limit-offset",
                        "request":{"limit":"limit","offset":"offset"},
                        "response":{"items":"/entries","next-offset":"/paging/count"},
                        "advance":"next-offset"
                    },
                    "listAuditEvents": {
                        "pattern":"cursor",
                        "request":{"cursor":"after"},
                        "response":{"items":"/events","next-cursor":"/next"}
                    }
                }}
            }))
            .unwrap(),
        ),
        ..Default::default()
    }
}

/// Every supported walker template emits through the typed lowering, and the
/// emitted package compiles when a JDK/Maven toolchain is available.
#[test]
fn manual_templates_emit_for_every_supported_shape() {
    let files = generate_document(manual_document(), &manual_options());
    let client = client_file(&files).content.clone();
    for expected in [
        // Page-number walk: one-based advance with a nested has-more stop.
        "private fun listRowsPageHasMore(page: ListRowsResponse200): Boolean?",
        "private fun listRowsPageItems(page: ListRowsResponse200): List<JsonNumber>?",
        "if (listRowsPageHasMore(data) == false) return null",
        " ?: JsonNumber.of(1L)",
        "return current.copy(page = Presence.Present(JsonNumber.of(advanced)))",
        // Top-level array page: the body itself is the collection.
        "private fun listBlobsPageItems(page: List<String>): List<String>?",
        "        return page\n    }",
        // Nested next-offset pointer feeding the offset control.
        "private fun listAlertsPageContinuation(page: ListAlertsResponse200): JsonNumber?",
        "return current.copy(offset = Presence.Present(token))",
        // Required cursor control: the stall guard compares the caller's token.
        "private fun listAuditEventsPageContinuation(page: ListAuditEventsResponse200): String?",
        "val previous = current.after",
        "if (token == previous) throw PaginationStalled(token, \"listAuditEvents\",",
        "return current.copy(after = token)",
    ] {
        assert!(
            client.contains(expected),
            "manual Client.kt is missing:\n{expected}\n--- emitted: ---\n{client}"
        );
    }

    // The manual package typechecks when the JVM toolchain is available.
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_pagination: no JDK 21 found; emission assertions only");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_pagination: no Maven found; emission assertions only");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, root.path()).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the manual mappings\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Native compile of the emitted package, when a JDK and Maven are available.
#[test]
fn emitted_package_compiles_with_a_jdk_toolchain() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_pagination: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_pagination: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(&configured_options()), root.path()).unwrap();
    let kotlin = root.path().join("kotlin");
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(&kotlin)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("kotlin_pagination: maven test-compile succeeded");
}

fn java_home() -> Option<std::path::PathBuf> {
    if let Some(home) =
        std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME").or_else(|| std::env::var_os("JAVA_HOME"))
    {
        return Some(std::path::PathBuf::from(home));
    }
    // The mise-managed Temurin install used by this workspace.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/java/temurin-21");
    let executable = home.join("bin/java");
    Command::new(&executable)
        .arg("-version")
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(home))
}

fn maven() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_KOTLIN_MAVEN") {
        return Some(std::path::PathBuf::from(path));
    }
    if Command::new("mvn")
        .arg("--version")
        .output()
        .is_ok_and(|probe| probe.status.success())
    {
        return Some(std::path::PathBuf::from("mvn"));
    }
    // mise-managed Maven installs carry a nested distribution directory.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/maven");
    let mut candidates = match std::fs::read_dir(&home) {
        Ok(installs) => installs
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| {
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .filter_map(|distribution| distribution.ok())
                    .map(|distribution| distribution.path().join("bin/mvn"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    candidates.sort();
    candidates.into_iter().find(|path| path.is_file())
}
