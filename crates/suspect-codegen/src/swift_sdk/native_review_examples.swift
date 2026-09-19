import Foundation
import XCTest
import GeneratedSDK

private actor Recording: HTTPTransport {
    private var count = 0
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        count += 1
        XCTAssertEqual(request.method, "POST")
        if count == 1 {
            XCTAssertEqual(request.url.absoluteString, "https://example.test/api/v1/echo/account?body=query-body&input=query-input&_parameter0=argument-zero&request_body=body-label")
            XCTAssertEqual(request.body, Data(#"{"text":"payload"}"#.utf8))
        } else {
            XCTAssertEqual(request.url.absoluteString, "https://example.test/api/v1/echo/direct?body=direct-body&input=direct-input&_parameter0=direct-zero&request_body=direct-label")
            XCTAssertEqual(request.body, Data(#"{"text":"direct-payload"}"#.utf8))
        }
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(#"{"ok":true}"#.utf8))
    }
    func calls() -> Int { count }
}

@MainActor
final class ExampleNamesTests: XCTestCase {
    func testCollidingSourceNamesCompileAndExecuteGeneratedExample() async throws {
        let recording = Recording()
        let client = Client(credentials: Credentials(apiKey: "review-fixture-token"), transport: recording)
        // Executes the same generated body used by the DocC call-site example.
        switch try await Examples.echoNames(client: client) {
        case .status200(let response): XCTAssertTrue(response.data.ok)
        }
        // Source-to-native public labels remain unchanged by local allocation.
        let input = EchoNamesInput(client: "direct", querybody: "direct-body", input: "direct-input", parameter0: "direct-zero", requestBody: "direct-label", body: EchoBody(text: "direct-payload"))
        switch try await client.echoNames(input) {
        case .status200(let response): XCTAssertTrue(response.data.ok)
        }
        let calls = await recording.calls()
        XCTAssertEqual(calls, 2)
    }
}
