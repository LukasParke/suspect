import Foundation
import XCTest
import GeneratedSDK

private actor Recorded: HTTPTransport {
    let response: HTTPResponse
    var requests: [HTTPRequest] = []
    init(_ response: HTTPResponse = HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(#"{"value":"recorded"}"#.utf8))) { self.response = response }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse { requests.append(request); return response }
    func last() throws -> HTTPRequest { try XCTUnwrap(requests.last) }
    func count() -> Int { requests.count }
}
private let binary = Data([0, 255, 13, 10, 128])
private actor TransportFailures {
    var failure: TransportError?
    func record(_ error: TransportError) { failure = error }
    func last() -> TransportError? { failure }
}
private struct TracedTransport: HTTPTransport {
    let failures: TransportFailures
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        do { return try await URLSessionTransport().send(request) }
        catch let error as TransportError { await failures.record(error); throw error }
    }
}
private func orderedBytes(short: Bool = false) -> Data {
    var value = Data("--ordered\r\nX-Slot: 1\r\nContent-Type: application/json; profile=meta\r\n\r\n{\"q\":\"meta\"}\r\n--ordered\r\nContent-Type: application/octet-stream\r\n\r\n".utf8)
    value.append(binary)
    if !short { value.append(Data("\r\n--ordered\r\nContent-Type: text/plain\r\n\r\nend\r\n--ordered\r\nX-Slot: 2\r\nContent-Type: text/plain\r\n\r\n2".utf8)) }
    value.append(Data("\r\n--ordered--\r\n".utf8)); return value
}
private func orderedInput(short: Bool = false) -> SendOrderedMultipartBody {
    SendOrderedMultipartBody(
        part1: SendOrderedMultipartPart1Part(value: Query(q: "meta"), headers: SendOrderedMultipartPart1Headers(xSlot: 1)),
        part2: HTTPPart(binary),
        part3: short ? .missing : .value(HTTPPart("end")),
        items: short ? [] : [SendOrderedMultipartItemPart(value: 2, headers: SendOrderedMultipartItemHeaders(xSlot: 2))]
    )
}
@MainActor
final class RemainingStandardTests: XCTestCase {
    func client(_ suffix: String = "/base", timeout: TimeInterval = 3) throws -> Client {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_NEXT_BASE"])
        return Client(options: ClientOptions(serverURL: base + suffix, timeout: timeout))
    }
    func testCustomMethodCaseIsPreservedOnTheActualWire() async throws {
        let client = try client()
        let copy = try await client.copyResource(); XCTAssertEqual(copy.data.value, "COPY")
        let mixed = try await client.mixedMethod(); XCTAssertEqual(mixed.data.value, "MiXeD")
        let ext = try await client.extensionMethod(); XCTAssertEqual(ext.data.value, "x-PING")
        let lower = try await client.lowerGet(); XCTAssertEqual(lower.data.value, "get")
        let get = try await client.mixedGet(); XCTAssertEqual(get.data.value, "GeT")
        let head = try await client.lowerHead(); XCTAssertEqual(head.data.value, "head", "a custom lowercase token must not acquire HEAD's body suppression")
        let post = try await client.mixedPost(MixedPostInput(body: Query(q: "payload"))); XCTAssertEqual(post.data.value, "pOsT")
        let reset = try await client.resetContent(); XCTAssertEqual(reset.data, HTTPNoContent()); XCTAssertEqual(reset.typedHeaders.xReset.raw, "1")
    }
    func testWholeJSONAndTextHaveNoNamePrefixAndExactlyOneEncodingPass() async throws {
        let client = try client()
        let json = try await client.wholeJSON(WholeJSONInput(criteria: Query(q: "a +雪", exact: .value(try JsonNumber("1.00e+3"))), id: "a/b"))
        XCTAssertEqual(json.data.value, "/base/query/json/a%2Fb?%7B%22exact%22%3A1.00e%2B3%2C%22q%22%3A%22a%20%2B%E9%9B%AA%22%7D")
        let missing = try await client.wholeText(); XCTAssertEqual(missing.data.value, "/base/query/text")
        let text = try await client.wholeText(WholeTextInput(text: .value("a=1&b=a+b %2F#雪")))
        XCTAssertEqual(text.data.value, "/base/query/text?a%3D1%26b%3Da%2Bb%20%252F%23%E9%9B%AA")
        let empty = try await client.wholeText(WholeTextInput(text: .value("")))
        XCTAssertEqual(empty.data.value, "/base/query/text?")
    }
    func testWholeFormsKeepContentEncodingAndPartCodecs() async throws {
        let client = try client()
        let fields = FormFields(bar: true, foo: "a + b", meta: .value(Query(q: "雪")), tags: .value(["a+b", "c d"]), additionalProperties: try JsonObject([("z", JsonInteger(7))]))
        let response = try await client.wholeForm(WholeFormInput(id: "x/y", fields: fields))
        XCTAssertEqual(response.data.value, "/base/query/form/x%2Fy?bar=true&foo=a+%2B+b&meta=%7B%22q%22%3A%22%E9%9B%AA%22%7D&tags=a%2Bb&tags=c%20d&z=7")
        let recorded = Recorded(); let local = Client(transport: recorded)
        do { _ = try await local.largeForm(LargeFormInput(fields: .init(value: .value(Array(repeating: "x", count: 10))))); XCTFail("repeated form names escaped URL budget") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation); XCTAssertEqual(error.json?.kind, .resourceLimit) }
        do { _ = try await local.wholeForm(WholeFormInput(id: "x", fields: FormFields(bar: true, foo: "x", tags: .value([])))); XCTFail("per-item cardinality ignored") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        do { _ = try await local.wholeJSON(WholeJSONInput(criteria: Query(q: ""), id: "x")); XCTFail("complete query schema ignored") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        let count = await recorded.count(); XCTAssertEqual(count, 0)
    }
    func testPositionalMixedPartsUseIndicesBytesHeadersAndRemainingItems() async throws {
        let raw = orderedBytes()
        let recording = Recorded(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "multipart/mixed; boundary=ordered")], body: raw))
        let output = try await Client(transport: recording).sendOrdered(SendOrderedInput(body: orderedInput()), options: RequestOptions(multipartBoundary: "ordered"))
        let request = try await recording.last(); XCTAssertEqual(request.body, raw, "independent literal MIME layout")
        XCTAssertEqual(output.data.part1.value.q, "meta"); XCTAssertEqual(output.data.part1.headers.xSlot.raw, "1")
        XCTAssertEqual(output.data.part2.value, binary); XCTAssertEqual(output.data.items.map(\.value), [2])
        XCTAssertFalse(String(decoding: raw, as: UTF8.self).contains("Content-Disposition"), "unnamed parts must not gain inferred names")
        let wire = try await client().sendOrdered(SendOrderedInput(body: orderedInput()), options: RequestOptions(multipartBoundary: "ordered"))
        XCTAssertEqual(wire.rawBody, raw); XCTAssertEqual(wire.data.part2.value, binary)
        let short = try await client().sendOrdered(SendOrderedInput(body: orderedInput(short: true)), options: RequestOptions(multipartBoundary: "ordered"))
        XCTAssertEqual(short.rawBody, orderedBytes(short: true)); XCTAssertEqual(short.data.items.count, 0)
        if case .missing = short.data.part3 {} else { XCTFail("missing suffix acquired a default") }
    }
    func testPositionalFormDataHonorsExplicitDispositionAndMIMEStyleValue() async throws {
        let body = SendOrderedFormMultipartBody(
            part1: SendOrderedFormMultipartPart1Part(value: "A", headers: SendOrderedFormMultipartPart1Headers(contentDisposition: "form-data; name=\"first\"")),
            part2: SendOrderedFormMultipartPart2Part(value: [1, 2], headers: SendOrderedFormMultipartPart2Headers(contentDisposition: "form-data; name=\"numbers\""))
        )
        let expected = Data("--form\r\nContent-Disposition: form-data; name=\"first\"\r\nContent-Type: text/plain\r\n\r\nA\r\n--form\r\nContent-Disposition: form-data; name=\"numbers\"\r\n\r\n1,2\r\n--form--\r\n".utf8)
        let response = try await client().sendOrderedForm(SendOrderedFormInput(body: body), options: RequestOptions(multipartBoundary: "form"))
        XCTAssertEqual(response.rawBody, expected); XCTAssertEqual(response.data.part2.value, [1, 2])
        var invalid = body; invalid.part1.headers.contentDisposition = "attachment; filename=\"x\""
        let recorded = Recorded()
        do { _ = try await Client(transport: recorded).sendOrderedForm(SendOrderedFormInput(body: invalid)); XCTFail("positional form-data lost its required name") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        let calls = await recorded.count(); XCTAssertEqual(calls, 0)
    }
    func testEmptyFalsePrefixAndEncodingBeyondSchemaPrefix() async throws {
        let empty = try await client().emptyParts(EmptyPartsInput(body: EmptyPartsMultipartBody()), options: RequestOptions(multipartBoundary: "empty"))
        XCTAssertEqual(empty.rawBody, Data("--empty--\r\n".utf8))
        let barrier = try await client().barrier(BarrierInput(body: BarrierMultipartBody(part1: .value(HTTPPart("one")))), options: RequestOptions(multipartBoundary: "stop"))
        if case .value(let part) = barrier.data.part1 { XCTAssertEqual(part.value, "one") } else { XCTFail("valid prefix lost") }
        let extended = try await client().extendedPrefix(ExtendedPrefixInput(body: ExtendedPrefixMultipartBody(part1: HTTPPart("one"), part2: .value(HTTPPart(2)))), options: RequestOptions(multipartBoundary: "extended"))
        if case .value(let part) = extended.data.part2 { XCTAssertEqual(part.value, 2) } else { XCTFail("prefixEncoding beyond prefixItems lost its items codec") }
        XCTAssertTrue(extended.rawBody.range(of: Data("Content-Type: application/json\r\n\r\n2".utf8)) != nil)
        let two = Data("--stop\r\nContent-Type: text/plain\r\n\r\none\r\n--stop\r\nContent-Type: text/plain\r\n\r\ntwo\r\n--stop--\r\n".utf8)
        let recorded = Recorded(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "multipart/mixed; boundary=stop")], body: two))
        do { _ = try await Client(transport: recorded).barrier(BarrierInput(body: .init())); XCTFail("false prefix allowed a later part") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .responseDecoding) }
    }
    func testPositionalGapsLimitsAndPerPositionCodecFailures() async throws {
        let recorded = Recorded(); let client = Client(transport: recorded)
        var gap = orderedInput(); gap.part3 = .missing
        do { _ = try await client.sendOrdered(SendOrderedInput(body: gap)); XCTFail("missing position before items") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        var oversized = orderedInput(); oversized.part2.value = Data(repeating: 255, count: 17)
        do { _ = try await client.sendOrdered(SendOrderedInput(body: oversized)); XCTFail("byte part declaration ignored") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        var tooMany = orderedInput(); tooMany.items = Array(repeating: tooMany.items[0], count: 3)
        do { _ = try await client.sendOrdered(SendOrderedInput(body: tooMany)); XCTFail("maxItems ignored") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation) }
        var wrongItem = orderedInput(); wrongItem.items[0].value = 10
        do { _ = try await client.sendOrdered(SendOrderedInput(body: wrongItem)); XCTFail("itemEncoding codec ignored") } catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        let count = await recorded.count(); XCTAssertEqual(count, 0)
        for raw in [Data("--ordered--\r\n".utf8), orderedBytes().replacingFirst(Data("X-Slot: 1\r\n".utf8), with: Data()), orderedBytes().replacingFirst(Data("{\"q\":\"meta\"}".utf8), with: Data("{\"q\":\"\"}".utf8))] {
            let r = Recorded(HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "multipart/mixed; boundary=ordered")], body: raw))
            do { _ = try await Client(transport: r).sendOrdered(SendOrderedInput(body: orderedInput())); XCTFail("invalid positional response accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, .responseDecoding) }
        }
    }
    func testExactMethodTransportFramingBudgetsAndSystemTrust() async throws {
        let client = try client()
        for mode in ["chunked", "close", "continue"] { let value = try await client.exactTransport(ExactTransportInput(mode: mode)); XCTAssertEqual(value.data.value, mode) }
        for (mode, kind) in [("redirect", SDKError.Kind.transport), ("large", .responseTooLarge), ("ambiguous", .transport)] {
            do { _ = try await client.exactTransport(ExactTransportInput(mode: mode)); XCTFail("invalid framing/policy accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, kind) }
        }
        do { _ = try await self.client(timeout: 0.1).exactTransport(ExactTransportInput(mode: "timeout")); XCTFail("deadline ignored") } catch let error as SDKError { XCTAssertEqual(error.kind, .timeout) }
        let task = Task { try await client.exactTransport(ExactTransportInput(mode: "cancel")) }
        try await Task.sleep(nanoseconds: 80_000_000); task.cancel()
        do { _ = try await task.value; XCTFail("native socket cancellation ignored") } catch is CancellationError {}
        let markers = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_NEXT_MARKERS"])
        let cancelledPath = markers + "/exact-cancel"
        for _ in 0..<100 where !FileManager.default.fileExists(atPath: cancelledPath) { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertTrue(FileManager.default.fileExists(atPath: cancelledPath))
        let tls = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_NEXT_TLS"])
        let failures = TransportFailures()
        do { _ = try await Client(transport: TracedTransport(failures: failures), options: ClientOptions(serverURL: tls + "/base", timeout: 3)).lowerGet(); XCTFail("untrusted certificate accepted") } catch let error as SDKError { XCTAssertEqual(error.kind, .transport) }
        guard case .network(let domain, _) = await failures.last() else { return XCTFail("expected a checked TLS failure") }
        XCTAssertEqual(domain, "TLS", "trust rejection must not be a missing-listener or DNS failure")
    }
    func testExactMethodStreamBreakAndCancellationReleaseSockets() async throws {
        let client = try client()
        let first = try await client.exactEvents(ExactEventsInput(mode: "break"))
        for try await event in first.data { XCTAssertEqual(event.data, "first"); break }
        let pending = try await client.exactEvents(ExactEventsInput(mode: "cancel"))
        let task = Task { for try await _ in pending.data {} }
        try await Task.sleep(nanoseconds: 30_000_000); task.cancel()
        do { try await task.value; XCTFail("stream cancellation ignored") } catch is CancellationError {}
        let root = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_NEXT_MARKERS"])
        for name in ["stream-break", "stream-cancel"] {
            let path = root + "/" + name
            for _ in 0..<100 where !FileManager.default.fileExists(atPath: path) { try await Task.sleep(nanoseconds: 10_000_000) }
            XCTAssertTrue(FileManager.default.fileExists(atPath: path), "socket was not released: \(name)")
        }
    }
}
private extension Data {
    func replacingFirst(_ needle: Data, with value: Data) -> Data {
        guard let range = range(of: needle) else { return self }
        var result = Data(prefix(upTo: range.lowerBound)); result.append(value); result.append(suffix(from: range.upperBound)); return result
    }
}
