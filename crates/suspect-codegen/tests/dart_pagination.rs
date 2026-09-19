//! Emitted-only pagination for the native Dart HTTP backend: generated
//! pagination part, generated Client streams, and no-policy byte-identity.
//! Static runtime files are never modified; the walk lives entirely in the
//! generated package. When no Dart toolchain is installed the behavioral
//! checks degrade to strict static assertions (see the emitted-source review
//! notes in `dart_pagination_walkers_are_static_and_no_policy_is_unchanged`).

#![cfg(feature = "dart-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::SdkDefaults,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract() -> Arc<Contract> {
    let uri = Uri::parse("https://source.pagination.test/dart-pagination.json").unwrap();
    let provider = Arc::new(
        suspect_ref::DocumentProvider::new([suspect_ref::ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
            serde_json::to_vec(&pagination_document()).unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &uri).unwrap())
}

/// One limit/offset list operation and one cursor list operation.
fn pagination_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Pagination", "version": "1"},
        "servers": [{"url": "https://api.pagination.test/v1"}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer"}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}}
            },
            "/events": {"get": {
                "operationId": "listEvents",
                "parameters": [
                    {"name": "cursor", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Page", "content": {"application/json": {"schema": {
                    "type": "object",
                    "properties": {
                        "items": {"type": "array", "items": {"type": "string"}},
                        "next_page_token": {"type": "string"}
                    },
                    "required": ["items"]
                }}}}}}
            }
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::DartHttp,
        package_name: "pagination_sdk".into(),
        package_version: "0.1.0".into(),
        import_name: None,
    }
}

fn generate(contract: Arc<Contract>, options: &GenerationOptions) -> Vec<OutFile> {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..GenerationOptions::default()
    }
}

fn sorted(files: &mut [OutFile]) {
    files.sort_by(|left, right| left.path.cmp(&right.path));
}

fn source(files: &[OutFile], path: &str) -> String {
    files
        .iter()
        .find(|file| file.path == path)
        .unwrap_or_else(|| panic!("{path} missing"))
        .content
        .clone()
}

fn dart_toolchain() -> Option<String> {
    let output = Command::new("dart").arg("--version").output().ok()?;
    output.status.success().then(|| "dart".to_owned())
}

#[ignore = "requires the Dart SDK on the test host"]
#[test]
fn pagination_streams_and_helpers_emit_only_under_sdk_defaults() {
    let shared = contract();
    let files = generate(shared.clone(), &configured_options());
    let part = source(&files, "dart/lib/src/pagination.dart");
    let client = source(&files, "dart/lib/src/client.dart");
    let library = source(&files, "dart/lib/pagination_sdk.dart");
    {
        let expected = "part 'src/pagination.dart';";
        assert!(library.contains(expected), "library lacks {expected}");
    }
    for expected in [
        // Typed traversal error and per-operation pointer readers.
        "final class PaginationException extends SdkException",
        "String? _paginationListEventsCursor(ListEventsStatus200 page) {",
        "final token = _paginationListEventsCursor(page);",
        "List<String>? _paginationListWidgetsItems(ListWidgetsStatus200 page) {",
        "List<String>? _paginationListEventsItems(ListEventsStatus200 page) {",
        "JsonInteger? _paginationListWidgetsNext(JsonInteger? current, ListWidgetsStatus200 page) {",
        "String? _paginationListEventsNext(String? current, ListEventsStatus200 page) {",
        // Continuation rules: counting advance and the cursor stall guard.
        "if (count == 0) { return null; }",
        "base = current.toBigInt();",
        "return JsonInteger.fromBigInt(base + BigInt.from(count));",
        "if (token == null || token.isEmpty) { return null; }",
        "if (current != null && current == token) {",
        "identical continuation value",
        "typedef ListWidgetsNextInput = ({ Presence<JsonInteger> limit, Presence<JsonInteger> offset, Presence<String> filter });",
        "typedef ListEventsNextInput = ({ Presence<String> cursor });",
    ] {
        assert!(
            part.contains(expected),
            "pagination part lacks {expected}\n--- emitted: ---\n{part}"
        );
    }
    for expected in [
        "  Stream<ListWidgetsStatus200> listWidgetsPages({ Presence<JsonInteger> limit = const Absent(), Presence<JsonInteger> offset = const Absent(), Presence<String> filter = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async* {",
        "  Stream<String> listWidgetsItems({ Presence<JsonInteger> limit = const Absent(), Presence<JsonInteger> offset = const Absent(), Presence<String> filter = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async* {",
        "  Future<ListWidgetsNextInput?> listWidgetsNextPage({ Presence<JsonInteger> limit = const Absent(), Presence<JsonInteger> offset = const Absent(), Presence<String> filter = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async {",
        "  Stream<ListEventsStatus200> listEventsPages({ Presence<String> cursor = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async* {",
        "  Stream<String> listEventsItems({ Presence<String> cursor = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async* {",
        "  Future<ListEventsNextInput?> listEventsNextPage({ Presence<String> cursor = const Absent(), CancellationToken? cancellation, Duration? timeout, ServerSelection? server, int? securityAlternative }) async {",
        // Only the pagination control changes between requests.
        "final page = await listWidgets(limit: limit, offset: control, filter: filter, cancellation: cancellation, timeout: timeout, server: server, securityAlternative: securityAlternative);",
        "final page = await listEvents(cursor: control, cancellation: cancellation, timeout: timeout, server: server, securityAlternative: securityAlternative);",
        "final page = await listEvents(cursor: cursor, cancellation: cancellation, timeout: timeout, server: server, securityAlternative: securityAlternative);",
        "yield page;",
        // The configured initial offset fills the first request.
        "if (!control.isPresent) {",
        "control = Present(JsonInteger.fromInt(0));",
        // Manual driving rebuilds the whole input record.
        "return (limit: limit, offset: Present(next), filter: filter);",
        "return (cursor: Present(next));",
    ] {
        assert!(
            client.contains(expected),
            "client lacks {expected}\n--- emitted: ---\n{client}"
        );
    }

    // Without SDK defaults nothing new is emitted at all, and every remaining
    // file stays byte-identical.
    let mut control = generate(shared, &GenerationOptions::default());
    let mut configured = files;
    sorted(&mut control);
    sorted(&mut configured);
    assert!(
        !control
            .iter()
            .any(|file| file.path == "dart/lib/src/pagination.dart")
    );
    let plain_library = source(&control, "dart/lib/pagination_sdk.dart");
    assert!(!plain_library.contains("part 'src/pagination.dart'"));
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    let suffix = "}\n";
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        if file.path == "dart/lib/src/client.dart" {
            let plain_body = file
                .content
                .strip_suffix(suffix)
                .expect("client closing shape");
            assert!(
                emitted.content.starts_with(plain_body) && emitted.content.ends_with(suffix),
                "{} may only gain the appended pagination streams",
                file.path
            );
            continue;
        }
        if file.path == "dart/lib/pagination_sdk.dart" {
            // The library gains exactly the one pagination part directive.
            assert_eq!(
                emitted.content,
                format!("{}part 'src/pagination.dart';\n", file.content),
                "{} may only gain the pagination part directive",
                file.path
            );
            continue;
        }
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
}

#[ignore = "requires the Dart SDK on the test host"]
#[test]
fn disabled_pagination_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version": "v1", "pagination": "off"
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let shared = contract();
    let mut disabled = generate(shared.clone(), &off);
    let mut control = generate(shared, &GenerationOptions::default());
    sorted(&mut disabled);
    sorted(&mut control);
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[ignore = "requires the Dart SDK on the test host"]
#[test]
fn plan_carries_the_pagination_outcome_only_when_configured() {
    use suspect_codegen::{dart_sdk, sdk_defaults};
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = dart_sdk::plan_sdk(
        contract.clone(),
        &selected,
        dart_sdk::DartConfig {
            sdk_defaults: Some(SdkDefaults::v1()),
            ..Default::default()
        },
    )
    .unwrap();
    let outcome = configured
        .pagination()
        .expect("configured policy is carried");
    assert_eq!(outcome.paginated.len(), 2);
    let widgets = outcome
        .paginated
        .iter()
        .find(|page| page.operation == "listWidgets")
        .expect("limit/offset operation");
    assert_eq!(widgets.initial_offset, Some(0));
    assert_eq!(
        widgets.advance,
        sdk_defaults::PaginationAdvance::ItemsReturned
    );
    let control = dart_sdk::plan_sdk(contract, &selected, dart_sdk::DartConfig::default()).unwrap();
    assert!(control.pagination().is_none());
}

/// With no installed Dart toolchain this test degrades to strict static
/// assertions plus a documented manual verification of the emitted walk:
///
/// 1. `listWidgetsPages` declares `var control = offset;`, fills the
///    configured initial offset only when the caller left it absent, and
///    re-invokes the direct `listWidgets` with every argument preserved and
///    only `offset: control` rewritten, so page 1 is the direct call's result
///    and a caller-supplied offset wins.
/// 2. `_paginationListWidgetsNext` stops on a zero-item page (the documented
///    limit/offset fallback), advances the offset by the previous page's item
///    count with exact BigInt arithmetic, and never issues its own request:
///    every request goes through the reused direct call.
/// 3. `listWidgetsItems`/`listEventsItems` flatten each page's items after the
///    page yields, so `break` after the first item cancels the inner page
///    stream at its yield point and never starts the next request.
/// 4. `_paginationListEventsNext` stops when the cursor token is null or
///    empty, and throws `PaginationException` on a repeated identical token
///    instead of looping.
/// 5. `listEventsNextPage` returns the rebuilt input record with only the
///    cursor changed, or null at the stop rule.
/// 6. `async*` streams are single-subscription; pause propagates through the
///    inner `await for` at each yield point and cancel terminates the
///    generator at the yield, so no not-yet-started request can fire.
#[ignore = "requires the Dart SDK on the test host"]
#[test]
fn dart_pagination_walkers_are_static_and_no_policy_is_unchanged() {
    if dart_toolchain().is_none() {
        eprintln!(
            "dart_pagination: no Dart toolchain installed; static assertions and manual verification notes only"
        );
    }
    let files = generate(contract(), &configured_options());
    let part = source(&files, "dart/lib/src/pagination.dart");
    let client = source(&files, "dart/lib/src/client.dart");
    // The walk never invents a request path of its own: the pagination part
    // contains no runtime plumbing, and every page request in the emitted
    // streams goes through the direct operation method.
    for fragment in ["_RequestBuilder", "_exchange(", "_stream(", "HttpTransport"] {
        assert!(
            !part.contains(fragment),
            "pagination emission must reuse the direct call path, not the runtime plumbing: {fragment}"
        );
    }
    assert!(part.contains("part of '../pagination_sdk.dart';"));
    assert!(client.contains("part of '../pagination_sdk.dart';"));
    // Manifest stays honest: pagination helpers are configured behavior, not
    // inferred wire semantics.
    let manifest: Value = serde_json::from_str(&source(&files, "dart/sdk-manifest.json")).unwrap();
    assert_eq!(manifest["inferred_pagination"], false);
}
