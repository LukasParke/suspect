import Foundation
import XCTest
import GeneratedSDK

private let widgetJSON = #"{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"child":{"label":"root"},"payload":{"kind":"standard","text":"plain"}}"#
private let updatedJSON = #"{"id":"w1","amount":0.0000000000000000001,"meta":"present","payload":{"kind":"standard","text":"plain"}}"#
private let listJSON = #"{"items":[{"id":"w2","amount":0.0000000000000000001,"payload":{"kind":"secure","vault":"vlt-1"}}]}"#

private struct Fixture: Sendable {
    let method: String
    let url: String
    let body: String?
    var status = 200
    var response = widgetJSON
    var contentType = "Application/JSON; charset=utf-8"
}
private actor Recording: HTTPTransport {
    var fixtures: [Fixture]
    var calls = 0
    init(_ fixtures: [Fixture]) { self.fixtures = fixtures }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        calls += 1
        guard !fixtures.isEmpty else { XCTFail("unexpected transport entry"); throw TransportError.invalidURL }
        let fixture = fixtures.removeFirst()
        XCTAssertEqual(request.method, fixture.method)
        XCTAssertEqual(request.url.absoluteString, fixture.url)
        XCTAssertEqual(request.body.map { String(decoding: $0, as: UTF8.self) }, fixture.body)
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "authorization" }.map(\.value), ["Bearer m2-key"])
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "accept" }.map(\.value), ["application/json"])
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "content-type" }.map(\.value), fixture.body == nil ? [] : ["application/json"])
        return HTTPResponse(status: fixture.status, headers: [HTTPHeader("Content-Type", fixture.contentType)], body: Data(fixture.response.utf8))
    }
}
private actor Sleeping: HTTPTransport {
    private var started = false
    private var waiters: [CheckedContinuation<Void, Never>] = []
    func waitUntilStarted() async {
        if started { return }
        await withCheckedContinuation { waiters.append($0) }
    }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        started = true
        for waiter in waiters { waiter.resume() }
        waiters = []
        try await Task.sleep(for: .seconds(60))
        throw TransportError.invalidURL
    }
}

@MainActor
final class NativeM2Tests: XCTestCase {
    func testExactJSONIsNotAFoundationFloatAdapter() throws {
        for token in ["9007199254740993.000000000000000001", "18446744073709551615", "-0.00e+9999999999999999999999999999", "1e-9999999999999999999999999999"] {
            let value = try JsonValue.parse(token)
            XCTAssertEqual(try value.encodedString(), token)
        }
        for token in ["", "+1", "01", "-01", ".1", "1.", "1e", "1e+", "NaN", "Infinity", "1,2", " 1"] { XCTAssertThrowsError(try JsonNumber(token), token) }
        for text in ["[1,]", "{\"a\":1,}", "{\"a\":1,\"\\u0061\":2}", "\"\\uD800\"", "\"\\uDC00\"", "\"\\uD800\\u0041\""] { XCTAssertThrowsError(try JsonValue.parse(text), text) }
        XCTAssertThrowsError(try JsonValue.parse(Data([34, 0xc0, 0xaf, 34])))
        XCTAssertEqual(try JsonValue.parse(#""\uD83D\uDE00""#), .string("😀"))
        XCTAssertEqual(try JsonValue.parse(#"{"é":1,"e\u0301":2}"#).encodedString(), "{\"e\u{301}\":2,\"é\":1}")
        XCTAssertThrowsError(try JsonValue.parse("[[[]]]", limits: JsonLimits(maxDepth: 2)))
        XCTAssertThrowsError(try JsonValue.parse("[1,2]", limits: JsonLimits(maxNodes: 2)))
        XCTAssertThrowsError(try JsonValue.string("too long").encoded(limits: JsonLimits(maxBytes: 2)))
        XCTAssertThrowsError(try JsonValue.parse("\"too long\"", limits: JsonLimits(maxBytes: 2)))
        XCTAssertThrowsError(try JsonNumber(String(repeating: "9", count: 65_537)))
        XCTAssertEqual(try JsonInteger("1000.000e-2").int64Value(), 10)
        XCTAssertThrowsError(try JsonInteger("0.1"))
        XCTAssertThrowsError(try JsonInteger("1e99999999999999999999").int64Value())
    }

    func testSourceBoundCodableAndMutableEncodeValidation() throws {
        var body = WidgetInput(name: "alpha", amount: .value(try JsonNumber("1.2300e+2")))
        let encoded = try SDKJSONEncoder().encode(body)
        XCTAssertEqual(String(decoding: encoded, as: UTF8.self), #"{"amount":1.2300e+2,"name":"alpha"}"#)
        let decoded = try SDKJSONDecoder().decode(WidgetInput.self, from: encoded)
        XCTAssertEqual(decoded, body)
        XCTAssertThrowsError(try JSONDecoder().decode(WidgetInput.self, from: encoded)) { error in
            XCTAssertEqual((error as? JsonError)?.kind, .sourceBoundary)
        }
        XCTAssertThrowsError(try JSONEncoder().encode(body)) { error in
            XCTAssertEqual((error as? JsonError)?.kind, .sourceBoundary)
        }
        body.name = ""
        XCTAssertThrowsError(try WidgetInput.codec.encode(body)) { error in
            let error = error as? ValidationError
            XCTAssertEqual(error?.kind, .invalid)
            XCTAssertEqual(error?.source.pointer, "/components/schemas/WidgetInput/properties/name/minLength")
            XCTAssertEqual(error?.instancePath, "/name")
        }
        XCTAssertThrowsError(try WidgetInput.codec.decode(Data(#"{}"#.utf8)))
        XCTAssertThrowsError(try WidgetInput.codec.decode(Data(#"{"name":null}"#.utf8)))
        XCTAssertThrowsError(try WidgetInput.codec.decode(Data(#"{"name":"x","amount":null}"#.utf8)))
        var collisions = WidgetInput(name: "x")
        collisions.additionalProperties["name"] = .string("override")
        XCTAssertThrowsError(try WidgetInput.codec.encode(collisions))
        let value = try Widget.codec.decode(Data(widgetJSON.utf8))
        XCTAssertEqual(value.meta, .null)
        XCTAssertEqual(value.amount.raw, "9007199254740993.000000000000000001")
        XCTAssertEqual(value.child.valueIfPresent?.label, "root")
        XCTAssertEqual(value.child.valueIfPresent?.child, .missing)
        XCTAssertThrowsError(try WidgetList.codec.encode(WidgetList(items: Array(repeating: value, count: 1000)), limits: JsonLimits(maxNodes: 8)))
        if case .standardPayload(let standard) = value.payload { XCTAssertEqual(standard.text, "plain") }
        else { XCTFail("wrong source-tagged union branch") }
        let absent = try Widget.codec.decode(Data(#"{"id":"x","amount":1,"payload":{"kind":"secure","vault":"v"}}"#.utf8))
        XCTAssertEqual(absent.meta, .missing)
        XCTAssertThrowsError(try Widget.codec.decode(Data(#"{"id":"x","amount":1,"payload":{"kind":"unknown","text":"x"}}"#.utf8)))
        var node = WidgetNode(label: "leaf")
        for _ in 0..<150 { node = WidgetNode(label: "next", child: .value(node)) }
        XCTAssertThrowsError(try WidgetNode.codec.encode(node)) { error in XCTAssertEqual((error as? JsonError)?.kind, .resourceLimit) }
    }

    func testFourOriginalOperationsUseExactSourceWires() async throws {
        let recording = Recording([
            Fixture(method: "POST", url: "https://m2.example.test/api/v1/widgets", body: #"{"name":"alpha"}"#),
            Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2", body: nil, response: listJSON),
            Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A", body: nil),
            Fixture(method: "PATCH", url: "https://m2.example.test/api/v1/widgets/w1", body: "{}", response: updatedJSON),
        ])
        let client = Client(credentials: Credentials(apiKey: "m2-key"), transport: recording)
        switch try await client.createWidget(CreateWidgetInput(body: WidgetInput(name: "alpha"))) {
        case .status200(let response):
            XCTAssertEqual(response.data.amount.raw, "9007199254740993.000000000000000001")
            XCTAssertEqual(String(decoding: response.rawBody, as: UTF8.self), widgetJSON)
        }
        switch try await client.listWidgets(ListWidgetsInput(tag: .value("a"), tags: .value(["x", "y"]), labels: .value(["a,b", "c"]), limit: .value(2))) {
        case .status200(let response):
            XCTAssertEqual(response.data.items.count, 1)
            XCTAssertEqual(response.data.items[0].meta, .missing)
            if case .securePayload(let payload) = response.data.items[0].payload { XCTAssertEqual(payload.vault, "vlt-1") }
            else { XCTFail("wrong union branch") }
        }
        switch try await client.getWidget(GetWidgetInput(widgetId: "a/b 雪!'()*")) { case .status200(let response): XCTAssertEqual(response.data.id, "w1") }
        switch try await client.updateWidget(UpdateWidgetInput(widgetId: "w1", body: WidgetPatch())) {
        case .status200(let response): XCTAssertEqual(response.data.meta, .value("present"))
        }
        let count = await recording.calls
        XCTAssertEqual(count, 4)
    }

    func testInvalidInputsNeverEnterTransportAndDeclaredErrorsStayTyped() async throws {
        let recording = Recording([])
        let client = Client(credentials: Credentials(apiKey: "m2-key"), transport: recording)
        do { _ = try await client.createWidget(CreateWidgetInput(body: WidgetInput(name: ""))); XCTFail("invalid input accepted") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "..")); XCTFail("dot path accepted") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        do { _ = try await client.listWidgets(ListWidgetsInput(limit: .value(1001))); XCTFail("invalid query accepted") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "ok"), options: RequestOptions(maxResponseBytes: Int.max)); XCTFail("ceiling raised") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        let count = await recording.calls
        XCTAssertEqual(count, 0)
        let denied = Recording([Fixture(method: "POST", url: "https://m2.example.test/api/v1/widgets", body: #"{"name":"alpha"}"#, status: 422, response: #"{"message":"rejected"}"#)])
        do {
            _ = try await Client(credentials: Credentials(apiKey: "m2-key"), transport: denied).createWidget(CreateWidgetInput(body: WidgetInput(name: "alpha")))
            XCTFail("declared API error accepted")
        } catch let error as CreateWidgetAPIError {
            switch error {
            case .status422(let response): XCTAssertEqual(response.data.message, "rejected"); XCTAssertEqual(response.status, 422)
            default: XCTFail("wrong declared error status")
            }
        }
    }

    func testResponseFailuresLimitsAndCancellation() async throws {
        for (fixture, expected) in [
            (Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets/x", body: nil, response: #"{"id":1}"#), SDKError.Kind.responseDecoding),
            (Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets/x", body: nil, status: 500, response: "undeclared"), .unexpectedResponse),
            (Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets/x", body: nil, contentType: "text/plain"), .responseDecoding),
        ] {
            let transport = Recording([fixture])
            do { _ = try await Client(credentials: Credentials(apiKey: "m2-key"), transport: transport).getWidget(GetWidgetInput(widgetId: "x")); XCTFail("bad response accepted") }
            catch let error as SDKError { XCTAssertEqual(error.kind, expected); XCTAssertEqual(error.status, fixture.status) }
        }
        let oversize = Recording([Fixture(method: "GET", url: "https://m2.example.test/api/v1/widgets/x", body: nil)])
        do { _ = try await Client(credentials: Credentials(apiKey: "m2-key"), transport: oversize).getWidget(GetWidgetInput(widgetId: "x"), options: RequestOptions(maxResponseBytes: 8)); XCTFail("oversize accepted") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .responseTooLarge) }
        let sleeping = Sleeping()
        let task = Task { try await Client(credentials: Credentials(apiKey: "m2-key"), transport: sleeping).getWidget(GetWidgetInput(widgetId: "x")) }
        await sleeping.waitUntilStarted()
        task.cancel()
        do { _ = try await task.value; XCTFail("cancelled request succeeded") }
        catch is CancellationError { /* native cancellation was preserved */ }
    }

    func testURLSessionRealSocketsBoundCaptureRedirectTimeoutAndCancellation() async throws {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_HTTP_BASE"])
        let markers = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_HTTP_MARKERS"])
        let client = Client(credentials: Credentials(apiKey: "m2-key"), options: ClientOptions(serverURL: base, timeout: 5))
        switch try await client.getWidget(GetWidgetInput(widgetId: "wire-check")) {
        case .status200(let response): XCTAssertEqual(response.data.amount.raw, "9007199254740993.000000000000000001")
        }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "redirect")); XCTFail("redirect was followed") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .transport) }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "oversized"), options: RequestOptions(maxResponseBytes: 32)); XCTFail("chunked oversized response accepted") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .responseTooLarge) }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "delay"), options: RequestOptions(timeout: 0.05)); XCTFail("timeout ignored") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .timeout) }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "declared-large")); XCTFail("declared error accepted") }
        catch let error as GetWidgetAPIError {
            switch error {
            case .status404(let response):
                XCTAssertEqual(response.data.message.count, 256)
                XCTAssertEqual(response.rawBody.count, 64)
                XCTAssertTrue(response.rawBodyTruncated)
            }
        }
        do { _ = try await client.getWidget(GetWidgetInput(widgetId: "unknown")); XCTFail("undeclared error accepted") }
        catch let error as SDKError {
            XCTAssertEqual(error.kind, .unexpectedResponse)
            XCTAssertEqual(error.body.count, 64)
            XCTAssertTrue(error.bodyTruncated)
            XCTAssertFalse(error.description.contains(String(repeating: "x", count: 8)))
        }
        let task = Task { try await client.getWidget(GetWidgetInput(widgetId: "cancelled")) }
        let marker = markers + "/cancel-received"
        for _ in 0..<400 {
            if FileManager.default.fileExists(atPath: marker) { break }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTAssertTrue(FileManager.default.fileExists(atPath: marker), "cancellation test must enter real I/O")
        let started = ContinuousClock.now
        task.cancel()
        do { _ = try await task.value; XCTFail("cancelled URLSession request succeeded") }
        catch is CancellationError { /* preserved through the operation boundary */ }
        XCTAssertLessThan(started.duration(to: .now), .seconds(1))
    }
}
