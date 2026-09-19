import Foundation
import XCTest
import GeneratedSDK

private actor Echo: HTTPTransport {
    var count = 0
    let invalid: Bool
    init(invalid: Bool = false) { self.invalid = invalid }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        count += 1
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")],
            body: invalid ? Data(#"{"amount":1,"mode":"card"}"#.utf8) : request.body ?? Data())
    }
    func calls() -> Int { count }
}
private func object(_ members: [(String, JsonValue)]) throws -> JsonValue { .object(try JsonObject(members)) }

@MainActor
final class V2CodecTests: XCTestCase {
    func testConditionalDependencyAndMutableExactNumbers() throws {
        var checkout = Checkout(amount: try JsonNumber("9007199254740993.000000000000001"), mode: .card, billing: .value("address"), card: .null)
        let bytes = try Checkout.codec.encode(checkout)
        XCTAssertEqual(try Checkout.codec.decode(bytes), checkout)
        XCTAssertTrue(String(decoding: bytes, as: UTF8.self).contains("9007199254740993.000000000000001"))
        checkout.billing = .missing
        XCTAssertThrowsError(try Checkout.codec.encode(checkout)) { error in
            let error = error as? ValidationError; XCTAssertEqual(error?.kind, .invalid)
            XCTAssertTrue(error?.source.pointer.hasSuffix("/dependentRequired/card") ?? false)
        }
        checkout.card = .missing
        XCTAssertThrowsError(try Checkout.codec.encode(checkout), "then requires the card member")
        checkout.mode = .cash
        XCTAssertNoThrow(try Checkout.codec.encode(checkout))
        checkout.card = .null
        XCTAssertThrowsError(try Checkout.codec.encode(checkout), "cash's selected else branch rejects card presence")
    }
    func testPatternOverlapsAndAdditionalExclusionsAreChecked() throws {
        var model = DynamicRecord(id: "id", additionalProperties: try JsonObject([
            ("n_positive", .number(try JsonNumber("1.00e0"))), ("s_label", .string("text"))
        ]))
        let data = try DynamicRecord.codec.encode(model); XCTAssertEqual(try DynamicRecord.codec.decode(data), model)
        model.additionalProperties["n_positive"] = .number(0)
        XCTAssertThrowsError(try DynamicRecord.codec.encode(model)) { error in
            XCTAssertTrue((error as? ValidationError)?.source.pointer.hasSuffix("/patternProperties/_positive$/minimum") ?? false)
        }
        model.additionalProperties["n_positive"] = .number(1)
        model.additionalProperties["n_fraction"] = .number(try JsonNumber("1e-400"))
        XCTAssertThrowsError(try DynamicRecord.codec.encode(model), "integer pattern cannot narrow through Double")
        model.additionalProperties["n_fraction"] = nil
        model.additionalProperties["other"] = .bool(true)
        XCTAssertThrowsError(try DynamicRecord.codec.encode(model), "unmatched extras must reach additionalProperties:false")
        XCTAssertThrowsError(try DynamicRecord.codec.decode(Data(#"{"id":"id","s_label":3}"#.utf8)))
    }
    func testPropertyNamesAndUnicodeKeyIdentity() throws {
        let both = UnicodeRecord(additionalProperties: try JsonObject([
            ("é", .number(1)), ("e\u{301}", .bool(true))
        ]))
        let bytes = try UnicodeRecord.codec.encode(both)
        let decoded = try UnicodeRecord.codec.decode(bytes)
        XCTAssertEqual(decoded.additionalProperties.count, 2)
        XCTAssertEqual(decoded.additionalProperties["é"], .number(1)); XCTAssertEqual(decoded.additionalProperties["e\u{301}"], .bool(true))
        let onlyDecomposed = UnicodeRecord(additionalProperties: try JsonObject([("e\u{301}", .bool(false))]))
        XCTAssertNoThrow(try UnicodeRecord.codec.encode(onlyDecomposed), "normalized equality must not activate the other dependency")
        let onlyComposed = UnicodeRecord(additionalProperties: try JsonObject([("é", .number(1))]))
        XCTAssertThrowsError(try UnicodeRecord.codec.encode(onlyComposed))
        let invalidName = DynamicRecord(id: "id", additionalProperties: try JsonObject([("s_bad-name", .string("text"))]))
        XCTAssertThrowsError(try DynamicRecord.codec.encode(invalidName)) { error in
            let error = error as? ValidationError
            XCTAssertTrue(error?.source.pointer.contains("propertyNames") ?? false)
            XCTAssertEqual(error?.instancePath, "/s_bad-name")
        }
    }
    func testDependentSchemaEvaluatesTheWholeObject() throws {
        var value = DependentRecord(enabled: .value(false))
        XCTAssertThrowsError(try DependentRecord.codec.encode(value), "false is present and dependentSchemas is not applied to that Boolean")
        value.peer = .value("ok")
        XCTAssertNoThrow(try DependentRecord.codec.encode(value))
        value.peer = .value("x")
        XCTAssertThrowsError(try DependentRecord.codec.encode(value))
        value.enabled = .missing
        XCTAssertNoThrow(try DependentRecord.codec.encode(value), "an absent trigger skips its schema")
    }
    func testCheckedCarrierRetainsTheCompleteInstanceAndRevalidatesMutation() throws {
        var carrier = ConditionalCarrier(value: try object([("tag", .string("a")), ("payload", .number(2))]))
        let encoded = try ConditionalCarrier.codec.encode(carrier)
        XCTAssertEqual(String(decoding: encoded, as: UTF8.self), #"{"payload":2,"tag":"a"}"#)
        XCTAssertEqual(try ConditionalCarrier.codec.decode(encoded), carrier)
        carrier.value = .array([.string("else")])
        XCTAssertEqual(try ConditionalCarrier.codec.decode(ConditionalCarrier.codec.encode(carrier)), carrier)
        carrier.value = .array([.number(1)])
        XCTAssertThrowsError(try ConditionalCarrier.codec.encode(carrier))
        carrier.value = try object([("tag", .string("a")), ("payload", .number(2)), ("extra", .null)])
        XCTAssertThrowsError(try ConditionalCarrier.codec.encode(carrier), "the carrier must not erase closed-object constraints")
    }
    func testReferenceAnnotationsReachTheOwningScope() throws {
        var scope = RefScope(value: try object([("known", .number(1))]))
        XCTAssertNoThrow(try RefScope.codec.encode(scope))
        scope.value = try object([("known", .number(1)), ("unknown", .string("extra"))])
        XCTAssertThrowsError(try RefScope.codec.encode(scope)) { error in
            XCTAssertTrue((error as? ValidationError)?.source.pointer.hasSuffix("/RefScope/unevaluatedProperties") ?? false)
        }
    }
    func testTupleContainsAndUnevaluatedItemCarrier() throws {
        var tuple = TupleEnvelope(value: [.string("label"), .number(try JsonNumber("1.00e0"))])
        XCTAssertEqual(try TupleEnvelope.codec.decode(TupleEnvelope.codec.encode(tuple)), tuple)
        tuple.value.append(.bool(false))
        XCTAssertThrowsError(try TupleEnvelope.codec.encode(tuple), "unmatched tail is unevaluated")
        tuple.value = [.string("label"), .number(1), .number(2)]
        XCTAssertThrowsError(try TupleEnvelope.codec.encode(tuple)) { error in
            XCTAssertTrue((error as? ValidationError)?.source.pointer.hasSuffix("/maxContains") ?? false)
        }
    }
    func testAnyOfRetainsValuesFromAllSuccessfulBranches() throws {
        let bytes = Data(#"{"alpha":1,"beta":"b"}"#.utf8)
        let value = try UnionEnvelope.codec.decode(bytes)
        guard case .variant1(let first) = value else { return XCTFail("native anyOf choice must remain deterministic") }
        XCTAssertEqual(first.alpha.raw, "1"); XCTAssertEqual(first.additionalProperties["beta"], .string("b"))
        XCTAssertEqual(try JsonValue.parse(UnionEnvelope.codec.encode(value)), try JsonValue.parse(bytes))
        XCTAssertThrowsError(try UnionEnvelope.codec.decode(Data(#"{"alpha":1,"beta":false}"#.utf8)), "a failed branch cannot annotate beta")
    }
    func testMissingNullAndNullableCarrierStayDistinct() throws {
        var value = PresenceModel(present: .null)
        XCTAssertEqual(String(decoding: try PresenceModel.codec.encode(value), as: UTF8.self), #"{"present":null}"#)
        value.optional = .null
        XCTAssertEqual(try PresenceModel.codec.decode(PresenceModel.codec.encode(value)).optional, .null)
        var box = CarrierBox()
        XCTAssertEqual(try CarrierBox.codec.encode(box), Data("{}".utf8))
        box.choice = .null
        XCTAssertEqual(try CarrierBox.codec.encode(box), Data(#"{"choice":null}"#.utf8))
        box.choice = .value(NullableCarrier(value: try object([("id", .number(1))])))
        XCTAssertEqual(try CarrierBox.codec.encode(box), Data(#"{"choice":{"id":1}}"#.utf8))
        box.choice = .value(NullableCarrier(value: .null))
        XCTAssertThrowsError(try CarrierBox.codec.encode(box), "present non-null carrier is not the null alternative")
    }
    func testPublicCodecBoundariesValidateBeforeAndAfterTransport() async throws {
        let echo = Echo(); let client = Client(transport: echo)
        let good = Checkout(amount: 1, mode: .cash)
        let result = try await client.roundTripCheckout(RoundTripCheckoutInput(body: good))
        XCTAssertEqual(result.data, good)
        _ = try await Examples.roundTripConditionalCarrier(client: client)
        do { _ = try await client.roundTripCheckout(RoundTripCheckoutInput(body: Checkout(amount: 1, mode: .card))); XCTFail("request condition was not checked") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        let calls = await echo.calls(); XCTAssertEqual(calls, 2)
        do { _ = try await Client(transport: Echo(invalid: true)).roundTripCheckout(RoundTripCheckoutInput(body: good)); XCTFail("response condition was not checked") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .responseDecoding) }
        let codable = try SDKJSONDecoder().decode(Checkout.self, from: SDKJSONEncoder().encode(good))
        XCTAssertEqual(codable, good)
    }
}
