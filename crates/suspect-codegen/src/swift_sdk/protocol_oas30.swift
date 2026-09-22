import Foundation
import XCTest
import GeneratedSDK

private actor Recording: HTTPTransport {
    var requests: [HTTPRequest] = []
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        XCTAssertEqual(request.method, "POST")
        XCTAssertEqual(request.url.absoluteString, "https://old.example.test/v0/legacy/a%2Fb")
        XCTAssertFalse(request.headers.contains { $0.name.lowercased() == "authorization" })
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/octet-stream")], body: Data([0, 255, 128]))
    }
    func bodies() -> [Data?] { requests.map(\.body) }
}

@MainActor
final class OpenAPI30ProtocolTests: XCTestCase {
    func testOAS30HTTPProjectionAndItsNormativeBinaryMarker() async throws {
        let recorder = Recording(); let client = Client(transport: recorder)
        let payload = OldPayload(name: "native", nullableValue: .null)
        let result = try await client.legacy(LegacyInput(id: "a/b", body: .json(payload)))
        XCTAssertEqual(result.data, Data([0, 255, 128]))
        _ = try await client.legacy(LegacyInput(id: "a/b", body: .bytes(Data([255, 0]))))
        let bodies = await recorder.bodies()
        XCTAssertEqual(bodies[0], Data(#"{"name":"native","nullableValue":null}"#.utf8))
        XCTAssertEqual(bodies[1], Data([255, 0]))
        do { _ = try await client.legacy(LegacyInput(id: "a/b", body: .json(OldPayload(name: "", nullableValue: .null)))); XCTFail("source minimum ignored") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
    }
}
