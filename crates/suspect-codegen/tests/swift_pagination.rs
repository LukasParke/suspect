//! Generated pagination for the Swift package: emission shape, plan carriage,
//! and native behavioral verification of the emitted page/item sequences over
//! a scripted transport. Static runtime files are never modified; the walk
//! lives entirely in the generated `Pagination.swift`.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};

use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::SdkDefaults,
    swift_sdk,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn document() -> Value {
    json!({
        "openapi":"3.1.0","info":{"title":"Pagination","version":"1"},
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
                    "type":"object","required":["data","total"],"properties":{
                        "data":{"type":"array","items":{"$ref":"#/components/schemas/Widget"}},
                        "total":{"type":"integer"},
                        "has_more":{"type":"boolean"}
                    }}}}}}}
            },
            "/things":{"get":{
                "operationId":"listThings",
                "parameters":[
                    {"name":"page","in":"query","schema":{"type":"integer","minimum":1}},
                    {"name":"limit","in":"query","schema":{"type":"integer","minimum":1}}
                ],
                "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{
                    "type":"object","properties":{
                        "data":{"type":"array","items":{"type":"string"}}
                    }}}}}}}
            },
            "/events":{"get":{
                "operationId":"listEvents",
                "parameters":[
                    {"name":"cursor","in":"query","schema":{"type":"string"}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
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

fn generate(options: &GenerationOptions) -> Vec<OutFile> {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "PaginationSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        options,
    )
    .unwrap()
}

fn configured_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults::v1()),
        ..Default::default()
    }
}

fn pagination_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "swift/Sources/PaginationSDK/Pagination.swift")
        .expect("generated Pagination.swift")
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn configured_emission_adds_only_pagination_swift() {
    let configured = generate(&configured_options());
    let control = generate(&GenerationOptions::default());
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("Pagination.swift")),
        "no-policy output must not carry pagination helpers"
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the configured policy may add exactly one file"
    );
    for file in &control {
        let emitted = configured
            .iter()
            .find(|candidate| candidate.path == file.path)
            .unwrap_or_else(|| panic!("{} disappeared under the policy", file.path));
        assert_eq!(emitted.content, file.content, "{} changed", file.path);
    }
    let pagination = pagination_file(&configured).content.clone();
    for expected in [
        // The typed traversal error.
        "public struct PaginationError: Error, Sendable, CustomStringConvertible",
        "pagination continuation repeated for \\(operation): \\(value)",
        // Limit/offset walk: exact integer advance with the documented
        // zero-item stop, the caller's filter preserved, and the configured
        // initial offset filled only when the caller left it absent.
        "public struct ListWidgetsPageSequence: AsyncSequence, AsyncIteratorProtocol, Sendable",
        "public struct ListWidgetsItemSequence: AsyncSequence, AsyncIteratorProtocol, Sendable",
        "public func listWidgetsPages(_ input: ListWidgetsInput = .init()) -> ListWidgetsPageSequence",
        "public func listWidgetsItems(_ input: ListWidgetsInput = .init()) -> ListWidgetsItemSequence",
        "public func listWidgetsNextPage(_ input: ListWidgetsInput = .init()) async throws -> ListWidgetsInput?",
        "func listWidgets_pageItems(_ page: ListWidgetsResponse200) -> [Widget]?",
        "return page.data",
        "rebuilt.offset = .value(JsonInteger(0))",
        "let advanced = offset.addingReportingOverflow(count)",
        "rebuilt.offset = .value(JsonInteger(advanced.partialValue))",
        "if count == 0 { return page }",
        // Cursor walk: pointer-driven continuation with the repeat guard, and
        // the empty-token stop rule.
        "public struct ListEventsPageSequence: AsyncSequence, AsyncIteratorProtocol, Sendable",
        "public struct ListEventsItemSequence: AsyncSequence, AsyncIteratorProtocol, Sendable",
        "public func listEventsPages(_ input: ListEventsInput = .init()) -> ListEventsPageSequence",
        "public func listEventsItems(_ input: ListEventsInput = .init()) -> ListEventsItemSequence",
        "public func listEventsNextPage(_ input: ListEventsInput = .init()) async throws -> ListEventsInput?",
        "func listEvents_pageItems(_ page: ListEventsResponse200) -> [String]?",
        "func listEvents_pageContinuation(_ page: ListEventsResponse200) -> String?",
        "page.nextPageToken",
        "if continuation.isEmpty { return page }",
        "throw PaginationError(operation: \"listEvents\", value: repeated)",
        "rebuilt.cursor = .value(continuation)",
        // Page-number walk: one-based increment with the documented stops.
        "public struct ListThingsPageSequence: AsyncSequence, AsyncIteratorProtocol, Sendable",
        "public func listThingsPages(_ input: ListThingsInput = .init()) -> ListThingsPageSequence",
        "func listThings_pageItems(_ page: ListThingsResponse200) -> [String]?",
        "let advanced = number.addingReportingOverflow(1)",
        "rebuilt.page = .value(JsonInteger(advanced.partialValue))",
        // The declared stop rules and laziness are documented on the walks.
        "The first call requests the input as given",
        "stops on a zero-item page",
    ] {
        assert!(
            pagination.contains(expected),
            "Pagination.swift is missing:\n{expected}\n--- emitted: ---\n{pagination}"
        );
    }
}

/// A manual mapping may declare a has-more indicator; the emitted walk must
/// then stop on `false` even when the page still returned items.
#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn manual_has_more_mapping_emits_and_honors_the_false_stop_rule() {
    let defaults: SdkDefaults = serde_json::from_value(json!({
        "version": "v1",
        "pagination": {
            "operations": {
                "listWidgets": {
                    "pattern": "limit-offset",
                    "request": {"limit": "limit", "offset": "offset"},
                    "response": {"items": "/data", "has-more": "/has_more"},
                    "initial_offset": 0,
                    "advance": "items-returned"
                }
            }
        }
    }))
    .unwrap();
    let files = generate(&GenerationOptions {
        sdk_defaults: Some(defaults),
        ..Default::default()
    });
    let pagination = pagination_file(&files).content.clone();
    assert!(
        pagination
            .contains("func listWidgets_pageHasMore(_ page: ListWidgetsResponse200) -> Bool?"),
        "{pagination}"
    );
    assert!(
        pagination.contains("func listWidgets_pageHasMore"),
        "{pagination}"
    );
    assert_eq!(
        pagination
            .matches("if listWidgets_pageHasMore(page.data) == false")
            .count(),
        1,
        "the walk must stop on a false has-more indicator"
    );
    // An explicit mapping does not disable detection for the other operations.
    assert!(
        pagination.contains("ListEventsPageSequence"),
        "{pagination}"
    );
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn policy_without_paginated_operations_emits_nothing_new() {
    let off = GenerationOptions {
        sdk_defaults: Some(
            serde_json::from_value(json!({
                "version":"v1","pagination":"off"
            }))
            .unwrap(),
        ),
        ..Default::default()
    };
    let disabled = generate(&off);
    let control = generate(&GenerationOptions::default());
    assert_eq!(disabled.len(), control.len());
    for (disabled, control) in disabled.iter().zip(control.iter()) {
        assert_eq!(disabled.path, control.path);
        assert_eq!(disabled.content, control.content);
    }
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn plan_carries_the_pagination_outcome_only_when_configured() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let configured = swift_sdk::plan_sdk(
        contract.clone(),
        &selected,
        swift_sdk::SwiftConfig {
            sdk_defaults: Some(serde_json::from_value(json!({"version":"v1"})).unwrap()),
            ..Default::default()
        },
    )
    .unwrap();
    let pagination = configured
        .pagination()
        .expect("configured policy is carried");
    assert_eq!(pagination.operations.len(), 3);
    assert_eq!(pagination.error_type, "PaginationError");
    let widgets = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern
                == suspect_codegen::sdk_defaults::PaginationPattern::LimitOffset
        })
        .expect("limit/offset operation");
    assert_eq!(widgets.selection.initial_offset, Some(0));
    assert_eq!(widgets.pages_method, "listWidgetsPages");
    assert_eq!(widgets.items_method, "listWidgetsItems");
    assert_eq!(widgets.page_sequence, "ListWidgetsPageSequence");
    assert_eq!(widgets.item_sequence, "ListWidgetsItemSequence");
    let things = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern
                == suspect_codegen::sdk_defaults::PaginationPattern::PageNumber
        })
        .expect("page-number operation");
    assert_eq!(things.pages_method, "listThingsPages");
    assert_eq!(things.item_sequence, "ListThingsItemSequence");
    let events = pagination
        .operations
        .iter()
        .find(|operation| {
            operation.selection.pattern == suspect_codegen::sdk_defaults::PaginationPattern::Cursor
        })
        .expect("cursor operation");
    assert_eq!(events.selection.initial_offset, None);
    assert_eq!(events.pages_method, "listEventsPages");
    let control =
        swift_sdk::plan_sdk(contract, &selected, swift_sdk::SwiftConfig::default()).unwrap();
    assert!(control.pagination().is_none());
}

const BEHAVIOR: &str = r##"import Foundation
import XCTest
import PaginationSDK

/// Scripted transport: records request URLs and replays queued JSON pages.
actor ScriptedTransport: HTTPTransport {
    private let pages: [String]
    private var requests: [String] = []

    init(pages: [String]) {
        self.pages = pages
    }

    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let index = requests.count
        requests.append(request.url.absoluteString)
        guard index < pages.count else {
            return HTTPResponse(status: 500, headers: [], body: Data())
        }
        return HTTPResponse(
            status: 200,
            headers: [HTTPHeader("Content-Type", "application/json")],
            body: Data(pages[index].utf8)
        )
    }

    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        throw TransportError.exactMethodUnavailable
    }

    func recorded() -> [String] {
        requests
    }
}

final class PaginationWalkTests: XCTestCase {
    private func client(_ transport: ScriptedTransport) -> Client {
        Client(transport: transport)
    }

    func testLimitOffsetWalkIssuesOffsetZeroThenTwoAndPreservesFilters() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"data":[{"id":"a"},{"id":"b"}],"total":9}"#,
            #"{"data":[],"total":9}"#,
        ])
        var walked = 0
        for try await page in client(transport).listWidgetsPages(
            .init(limit: .value(JsonInteger(2)), filter: .value("x"))
        ) {
            walked += 1
            XCTAssertEqual(page.status, 200)
            let expected = walked == 1 ? ["a", "b"] : Array<String>()
            XCTAssertEqual(page.data.data.map(\.id), expected)
        }
        XCTAssertEqual(walked, 2)
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2, "\(requests)")
        for request in requests {
            XCTAssertTrue(request.contains("limit=2"), request)
            XCTAssertTrue(request.contains("filter=x"), request)
        }
        XCTAssertTrue(requests[0].contains("offset=0"), requests[0])
        XCTAssertTrue(requests[1].contains("offset=2"), requests[1])
    }

    func testItemWalkFlattensPagesAndHonorsCallerOffset() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"data":[{"id":"a"},{"id":"b"}],"total":9}"#,
            #"{"data":[{"id":"c"}],"total":9}"#,
            #"{"data":[],"total":9}"#,
        ])
        var items: [String] = []
        for try await item in client(transport).listWidgetsItems(
            .init(limit: .value(JsonInteger(2)), offset: .value(JsonInteger(10)), filter: .value("x"))
        ) {
            items.append(item.id)
        }
        XCTAssertEqual(items, ["a", "b", "c"])
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 3, "\(requests)")
        XCTAssertTrue(requests[0].contains("offset=10"), requests[0])
        XCTAssertTrue(requests[1].contains("offset=12"), requests[1])
        XCTAssertTrue(requests[2].contains("offset=13"), requests[2])
    }

    func testEarlyBreakIssuesNoSecondRequest() async throws {
        let pagesTransport = ScriptedTransport(pages: [
            #"{"data":[{"id":"a"}],"total":9}"#,
            #"{"data":[{"id":"b"}],"total":9}"#,
        ])
        for try await _ in client(pagesTransport).listWidgetsPages(.init()) {
            break
        }
        let afterPages = await pagesTransport.recorded()
        XCTAssertEqual(afterPages.count, 1)

        let itemsTransport = ScriptedTransport(pages: [
            #"{"data":[{"id":"a"}],"total":9}"#,
            #"{"data":[{"id":"b"}],"total":9}"#,
        ])
        var items: [String] = []
        for try await item in client(itemsTransport).listWidgetsItems(.init()) {
            items.append(item.id)
            break
        }
        XCTAssertEqual(items, ["a"])
        let afterItems = await itemsTransport.recorded()
        XCTAssertEqual(afterItems.count, 1)
    }

    func testCursorWalkSendsSecondCursorAndStopsWhenMissing() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"items":["i1"],"next_page_token":"c2"}"#,
            #"{"items":["i2"]}"#,
        ])
        var items: [String] = []
        for try await item in client(transport).listEventsItems(.init(filter: .value("f"))) {
            items.append(item)
        }
        XCTAssertEqual(items, ["i1", "i2"])
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2, "\(requests)")
        XCTAssertFalse(requests[0].contains("cursor="), requests[0])
        XCTAssertTrue(requests[1].contains("cursor=c2"), requests[1])
    }

    func testCallerCursorIsHonoredForTheFirstPageThenReplaced() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"items":["i1"],"next_page_token":"c9"}"#,
            #"{"items":["i2"]}"#,
        ])
        var sequence = client(transport).listEventsPages(.init(cursor: .value("c1")))
        _ = try await sequence.next()
        _ = try await sequence.next()
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 2, "\(requests)")
        XCTAssertTrue(requests[0].contains("cursor=c1"), requests[0])
        XCTAssertTrue(requests[1].contains("cursor=c9"), requests[1])
    }

    func testEmptyCursorTokenStopsTheWalk() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"items":["i1"],"next_page_token":""}"#,
        ])
        var pages = 0
        for try await _ in client(transport).listEventsPages() {
            pages += 1
        }
        XCTAssertEqual(pages, 1)
        let recorded = await transport.recorded()
        XCTAssertEqual(recorded.count, 1)
    }

    func testRepeatedCursorFailsWithTheTypedPaginationError() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"items":["i1"],"next_page_token":"c1"}"#,
            #"{"items":["i2"],"next_page_token":"c1"}"#,
        ])
        var walked = 0
        do {
            for try await _ in client(transport).listEventsPages() {
                walked += 1
            }
            XCTFail("a repeated continuation must fail the walk")
        } catch is PaginationError {
            // The page carrying the repeated token is still delivered exactly
            // once; the typed error surfaces on the following pull.
        } catch {
            XCTFail("unexpected error: \(error)")
        }
        XCTAssertEqual(walked, 2)
        let recorded = await transport.recorded()
        XCTAssertEqual(recorded.count, 2, "non-progress looped")
    }

    func testCancellationStopsTheWalkBeforeAnyRequest() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"items":["i1"],"next_page_token":"c2"}"#,
        ])
        // Hold the walk until the test has cancelled its task, so the
        // cancellation check fires before any transport work.
        let (stream, gate) = AsyncStream<Void>.makeStream()
        let walk = client(transport)
        let task = Task {
            for await _ in stream {}
            do {
                for try await _ in walk.listEventsPages() {}
                XCTFail("cancelled walk fetched a page")
            } catch is CancellationError {
            } catch {
                XCTFail("unexpected error: \(error)")
            }
        }
        task.cancel()
        gate.finish()
        await task.value
        let recorded = await transport.recorded()
        XCTAssertEqual(recorded.count, 0)
    }

    func testPageNumberWalkIncrementsThePageAndStopsOnEmptyPages() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"data":["t1","t2"]}"#,
            #"{"data":["t3"]}"#,
            #"{"data":[]}"#,
        ])
        var items: [String] = []
        for try await item in client(transport).listThingsItems(.init(limit: .value(JsonInteger(2)))) {
            items.append(item)
        }
        XCTAssertEqual(items, ["t1", "t2", "t3"])
        let requests = await transport.recorded()
        XCTAssertEqual(requests.count, 3, "\(requests)")
        XCTAssertFalse(requests[0].contains("page="), requests[0])
        XCTAssertTrue(requests[1].contains("page=2"), requests[1])
        XCTAssertTrue(requests[2].contains("page=3"), requests[2])
    }

    func testNextPageBuildsTheFollowingInput() async throws {
        let transport = ScriptedTransport(pages: [
            #"{"data":[{"id":"a"},{"id":"b"}],"total":9}"#,
        ])
        let next = try await client(transport).listWidgetsNextPage(
            .init(limit: .value(JsonInteger(2)), filter: .value("x"))
        )
        let builderRequests = await transport.recorded()
        XCTAssertEqual(builderRequests.count, 1, "builder issued extra requests")
        XCTAssertEqual(next?.offset.valueIfPresent?.raw, "2")
        XCTAssertEqual(next?.limit.valueIfPresent?.raw, "2")
        XCTAssertEqual(next?.filter.valueIfPresent, "x")

        // The zero-item page ends the walk with nil.
        let ended = ScriptedTransport(pages: [#"{"data":[],"total":9}"#])
        let endedInput = try await client(ended).listWidgetsNextPage(.init(limit: .value(JsonInteger(2))))
        XCTAssertNil(endedInput)
        let recordedEnded = await ended.recorded()
        XCTAssertEqual(recordedEnded.count, 1)

        // An absent continuation token ends the cursor walk with nil.
        let events = ScriptedTransport(pages: [#"{"items":["i1"]}"#])
        let eventsInput = try await client(events).listEventsNextPage(.init(cursor: .value("c2")))
        XCTAssertNil(eventsInput)
        let recordedEvents = await events.recorded()
        XCTAssertEqual(recordedEvents.count, 1)
    }
}
"##;

fn swiftc() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_SWIFTC_BIN") {
        return Some(std::path::PathBuf::from(path));
    }
    let found = Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .ok()?;
    if !found.status.success() {
        return None;
    }
    let text = String::from_utf8(found.stdout).ok()?;
    let path = std::path::PathBuf::from(text.trim());
    Command::new(&path).arg("--version").output().ok()?;
    Some(path)
}

fn swift() -> std::path::PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/usr/bin/swift"))
}

fn swift_command(root: &Path, action: &str) -> Command {
    let mut command = Command::new(swift());
    command.arg(action);
    if action == "test" {
        command.arg("--disable-swift-testing");
    }
    if let Some(sdkroot) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("--sdk").arg(&sdkroot).env("SDKROOT", sdkroot);
    }
    command.env("SWIFT_EXEC", swiftc().unwrap_or_else(|| "swiftc".into()));
    command.current_dir(root);
    command
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn native_walks_count_requests_and_preserve_input() {
    let Some(swiftc) = swiftc() else {
        eprintln!("swift_pagination: no Swift 6 toolchain; degrading to static assertions");
        return;
    };
    eprintln!("swift_pagination: {}", swiftc.display());
    let root = tempfile::tempdir().unwrap();
    let files = generate(&configured_options());
    suspect_codegen::write_files(&files, &root.path().join("sdk")).unwrap();

    // The generated package must build cleanly with warnings as errors.
    let build = swift_command(&root.path().join("sdk/swift"), "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/sdk"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "generated package build failed\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );

    // Behavioral consumer: a scripted transport drives the two stop rules and
    // the repeat guard without any loopback socket.
    let consumer = root.path().join("consumer");
    std::fs::create_dir_all(consumer.join("Tests/PaginationConsumer")).unwrap();
    std::fs::write(
        consumer.join("Package.swift"),
        "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"PaginationConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.testTarget(name: \"PaginationConsumer\", dependencies: [.product(name: \"PaginationSDK\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n",
    )
    .unwrap();
    std::fs::write(
        consumer.join("Tests/PaginationConsumer/PaginationTests.swift"),
        BEHAVIOR,
    )
    .unwrap();
    let test = swift_command(&consumer, "test")
        .arg("--scratch-path")
        .arg(root.path().join("build/consumer"))
        .args(["-Xswiftc", "-warnings-as-errors"])
        .output()
        .unwrap();
    assert!(
        test.status.success(),
        "native pagination behavior failed\n{}\n{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    eprintln!(
        "swift_pagination: {}",
        String::from_utf8_lossy(&test.stdout).trim()
    );
}
