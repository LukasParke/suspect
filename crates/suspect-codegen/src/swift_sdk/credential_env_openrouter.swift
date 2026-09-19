import Foundation
import Darwin
import XCTest
import OpenRouter

private let currentJSON = #"{"data":{"label":"current-fixture","limit":100,"usage":25.5000000000000001,"usage_daily":1,"usage_weekly":2,"usage_monthly":3,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"is_free_tier":false,"is_management_key":false,"is_provisioning_key":false,"limit_remaining":74.5,"limit_reset":"monthly","include_byok_in_limit":false,"creator_user_id":null,"rate_limit":{"requests":-1,"interval":"1h","note":"deprecated"}}}"#

private actor Capture: HTTPTransport {
    private var requests: [HTTPRequest] = []
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        XCTAssertEqual(request.method, "GET")
        XCTAssertTrue(["https://openrouter.ai/api/v1/key", "https://openrouter.ai/api/v1/credits"].contains(request.url.absoluteString))
        let body = request.url.path.hasSuffix("/key") ? currentJSON : #"{"data":{"total_credits":100.50,"total_usage":25.75}}"#
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(body.utf8))
    }
    func all() -> [HTTPRequest] { requests }
}

@MainActor
final class OpenRouterEnvironmentTests: XCTestCase {
    func testActualCurrentKeyIsTheDefaultReadOnlyCallAndUsesSourceHTTPS() async throws {
        XCTAssertEqual(Darwin.setenv("OPENROUTER_API_KEY", "controlled-user-token", 1), 0)
        let capture = Capture()
        let client = Client(transport: capture, options: ClientOptions(timeout: 15))
        let response = try await client.getCurrentKey()
        XCTAssertEqual(response.status, 200)
        XCTAssertEqual(response.data.data.label, "current-fixture")
        XCTAssertFalse(response.data.data.isManagementKey)
        XCTAssertEqual(response.data.data.usage.raw, "25.5000000000000001")
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests[0].url.absoluteString, "https://openrouter.ai/api/v1/key")
        XCTAssertEqual(requests[0].headers.filter { $0.name.lowercased() == "authorization" }.map(\.value), ["Bearer controlled-user-token"])
    }

    func testCreditsRemainsAnExplicitManagementOperation() async throws {
        XCTAssertEqual(Darwin.setenv("OPENROUTER_API_KEY", "controlled-management-token", 1), 0)
        let capture = Capture(); let client = Client.fromEnvironment(transport: capture)
        let response = try await client.getCredits()
        XCTAssertEqual(response.status, 200)
        XCTAssertEqual(response.data.data.totalCredits.raw, "100.50")
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests[0].url.absoluteString, "https://openrouter.ai/api/v1/credits")
    }

    func testMissingEmptyAndExplicitCredentialsOnTheActualCurrentKeySurface() async throws {
        let capture = Capture()
        for value in [String?.none, ""] {
            if let value { XCTAssertEqual(Darwin.setenv("OPENROUTER_API_KEY", value, 1), 0) }
            else { XCTAssertEqual(Darwin.unsetenv("OPENROUTER_API_KEY"), 0) }
            let client = Client.fromEnvironment(transport: capture)
            do { _ = try await client.getCurrentKey(); XCTFail("missing credentials reached transport") }
            catch let error as SDKError { XCTAssertEqual(error.kind, .requestRepresentation); XCTAssertTrue(error.headers.isEmpty); XCTAssertTrue(error.body.isEmpty) }
        }
        XCTAssertEqual(Darwin.setenv("OPENROUTER_API_KEY", "ignored-environment-token", 1), 0)
        let client = Client(credentials: Credentials(apiKey: "explicit-user-token"), transport: capture)
        _ = try await client.getCurrentKey()
        let requests = await capture.all(); XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests[0].headers.filter { $0.name.lowercased() == "authorization" }.map(\.value), ["Bearer explicit-user-token"])
    }
}
