import Foundation
import XCTest
import GeneratedSDK

private actor Echo: HTTPTransport {
    private var count = 0
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        count += 1
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: request.body ?? Data())
    }
    func calls() -> Int { count }
}
private func object(_ members: [(String, JsonValue)]) throws -> JsonValue { .object(try JsonObject(members)) }

@MainActor
final class V3CodecTests: XCTestCase {
    private var base: String { ProcessInfo.processInfo.environment["SUSPECT_SWIFT_RESOURCE_BASE"]! }

    func testDynamicTreeKeepsExactValuesAndRevalidatesMutation() throws {
        let exact = try JsonNumber("9007199254740993.000000000000001")
        var tree = Strict(value: try object([("children", .array([try object([("data", .number(exact))])]))]))
        let encoded = try Strict.codec.encode(tree)
        XCTAssertEqual(try Strict.codec.decode(encoded), tree)
        XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains(exact.raw))
        tree.value = try object([("children", .array([try object([("unexpected", .number(1))])]))])
        XCTAssertThrowsError(try Strict.codec.encode(tree)) { error in
            let failure = error as? ValidationError
            XCTAssertEqual(failure?.kind, .invalid)
            XCTAssertEqual(failure?.source.document, self.base + "/specs/releases/api.json")
            XCTAssertEqual(failure?.source.pointer, "/components/schemas/Strict/unevaluatedProperties")
            XCTAssertEqual(failure?.instancePath, "/children/0/unexpected")
        }
        XCTAssertNoThrow(try Tree.codec.encode(Tree(value: tree.value)), "unentered strict candidate must be inert")
    }

    func testDynamicNullabilityAndMissingRemainDistinct() throws {
        let missing = Choice(value: .object(.init()))
        let null = Choice(value: try object([("selected", .null)]))
        let number = Choice(value: try object([("selected", .number(7))]))
        XCTAssertEqual(try Choice.codec.encode(missing), Data("{}".utf8))
        XCTAssertEqual(try Choice.codec.encode(null), Data(#"{"selected":null}"#.utf8))
        XCTAssertNotEqual(missing, null)
        for value in [missing, null, number] { XCTAssertEqual(try Choice.codec.decode(Choice.codec.encode(value)), value) }
        XCTAssertThrowsError(try Choice.codec.encode(Choice(value: try object([("selected", .string("static fallback"))]))))
        XCTAssertThrowsError(try Choice.codec.decode(Data(#"{"selected":"static fallback"}"#.utf8)))
    }

    func testDynamicUnionUsesOwningContextWithoutStaticBranchNarrowing() throws {
        for value in [JsonValue.number(7), .bool(true)] {
            let carrier = DynamicUnion(value: value)
            XCTAssertEqual(try DynamicUnion.codec.decode(DynamicUnion.codec.encode(carrier)), carrier)
        }
        XCTAssertThrowsError(try DynamicUnion.codec.encode(DynamicUnion(value: .string("fallback"))))
        let carrier = DynamicUnion(value: .number(7))
        XCTAssertEqual(try SDKJSONDecoder().decode(DynamicUnion.self, from: SDKJSONEncoder().encode(carrier)), carrier)
        XCTAssertThrowsError(try JSONEncoder().encode(carrier))
    }

    func testNestedSourceCodecEntersItsResourceWithoutEvaluatingRoot() throws {
        let carrier = DynamicSlot(value: .number(7))
        XCTAssertEqual(try Codecs.roundTripDetachedRequest.encode(carrier), Data("7".utf8))
        XCTAssertEqual(try Codecs.roundTripDetachedRequest.decode(Data("7".utf8)), carrier)
        XCTAssertThrowsError(try Codecs.roundTripDetachedRequest.decode(Data(#""fallback""#.utf8))) { error in
            XCTAssertEqual((error as? ValidationError)?.source.document, "https://cdn.swift.test/detached.json")
            XCTAssertEqual((error as? ValidationError)?.source.pointer, "/$defs/Override/type")
        }
        // Starting at the independently exposed source has a genuinely different
        // resource context. Its codec must not be substituted inside the owner.
        XCTAssertThrowsError(try DynamicSlot.codec.encode(carrier))
        XCTAssertNoThrow(try DynamicSlot.codec.encode(DynamicSlot(value: .string("fallback"))))
    }

    func testStaticResourcesRetainTypedFieldsAndPhysicalErrorSources() throws {
        var record = Record(amount: try JsonInteger("9007199254740993.00"), id: "r", note: .null)
        XCTAssertEqual(try Record.codec.decode(Record.codec.encode(record)), record)
        record.note = .missing
        XCTAssertFalse(String(decoding: try Record.codec.encode(record), as: UTF8.self).contains("note"))
        record.amount = 9007199254740992
        XCTAssertThrowsError(try Record.codec.encode(record)) { error in
            XCTAssertEqual((error as? ValidationError)?.source.document, self.base + "/specs/releases/api.json")
            XCTAssertEqual((error as? ValidationError)?.source.pointer, "/components/schemas/Record/$defs/Threshold/minimum")
        }
        XCTAssertThrowsError(try Codecs.roundTripEscapedRequest.decode(Data("1".utf8))) { error in
            XCTAssertEqual((error as? ValidationError)?.source.document, "https://cdn.swift.test/releases/escaped.json")
            XCTAssertEqual((error as? ValidationError)?.source.pointer, "/$defs/a~1b~0% #é/minimum")
        }
    }

    func testExecutableExamplesAndRequestFailuresAtPublicBoundaries() async throws {
        let echo = Echo()
        let client = Client(transport: echo)
        _ = try await Examples.roundTripStrictTree(client: client)
        _ = try await Examples.roundTripChoice(client: client)
        _ = try await Examples.roundTripDynamicUnion(client: client)
        _ = try await Examples.roundTripDetached(client: client)
        do { _ = try await client.roundTripChoice(.init(body: Choice(value: try object([("selected", .string("bad"))])))); XCTFail("invalid request reached transport") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation); XCTAssertEqual(error.validation?.kind, .invalid) }
        do { _ = try await client.triggerResourceCycle(.init(body: Cycle(value: .null))); XCTFail("cycle was hidden") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation); XCTAssertEqual(error.validation?.kind, .evaluationFailure) }
        let calls = await echo.calls(); XCTAssertEqual(calls, 4)
    }

    func testDefaultTransportMakesResourceBoundWireCallsAndChecksResponses() async throws {
        let client = Client()
        let strict = Strict(value: try object([("children", .array([try object([("data", .number(1))])]))]))
        let strictResult = try await client.roundTripStrictTree(.init(body: strict))
        XCTAssertEqual(strictResult.data, strict)
        let choice = Choice(value: try object([("selected", .null)]))
        let choiceResult = try await client.roundTripChoice(.init(body: choice))
        XCTAssertEqual(choiceResult.data, choice)
        let union = DynamicUnion(value: .number(7))
        let unionResult = try await client.roundTripDynamicUnion(.init(body: union))
        XCTAssertEqual(unionResult.data, union)
        let record = Record(amount: 9007199254740993, id: "r")
        let recordResult = try await client.roundTripRecord(.init(body: record))
        XCTAssertEqual(recordResult.data, record)
        let detached = DynamicSlot(value: .number(7))
        let detachedResult = try await client.roundTripDetached(.init(body: detached))
        XCTAssertEqual(detachedResult.data, detached)
        let number = try JsonNumber("9007199254740993.000000000000001")
        let escapedResult = try await client.roundTripEscaped(.init(body: number))
        XCTAssertEqual(escapedResult.data.raw, number.raw)
        do { _ = try await client.roundTripStrictTree(.init(body: strict), options: .init(serverURL: base + "/invalid")); XCTFail("invalid response accepted") }
        catch let error as SDKError {
            XCTAssertEqual(error.kind, .responseDecoding)
            XCTAssertEqual(error.validation?.kind, .invalid)
            XCTAssertEqual(error.validation?.source.pointer, "/components/schemas/Strict/unevaluatedProperties")
        }
    }
}
