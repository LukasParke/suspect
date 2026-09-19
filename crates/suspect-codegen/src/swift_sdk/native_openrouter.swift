import Foundation
import XCTest
import GeneratedSDK

private typealias CreateBody = __CREATE_BODY__
private typealias UpdateBody = __UPDATE_BODY__
private let keyHash = "f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943"
private let createJSON = #"{"data":{"hash":"f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943","name":"Native Test Key","label":"Native Test Key","disabled":false,"limit":50.250,"limit_remaining":50.250,"limit_reset":"monthly","include_byok_in_limit":true,"usage":0,"usage_daily":0,"usage_weekly":0,"usage_monthly":0,"byok_usage":0,"byok_usage_daily":0,"byok_usage_weekly":0,"byok_usage_monthly":0,"created_at":"2026-09-08T10:30:00Z","updated_at":null,"external_user":null,"creator_user_id":null,"workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"},"key":"sk-or-v1-fixture-only"}"#
private let updateJSON = #"{"data":{"hash":"f01d52606dc8f0a8303a7b5cc3fa07109c2e346cec7c0a16b40de462992ce943","name":"Updated Native Key","label":"Updated Native Key","disabled":true,"limit":75.50,"limit_remaining":49.5,"limit_reset":"daily","include_byok_in_limit":true,"usage":25.5,"usage_daily":25.5,"usage_weekly":25.5,"usage_monthly":25.5,"byok_usage":17.38,"byok_usage_daily":17.38,"byok_usage_weekly":17.38,"byok_usage_monthly":17.38,"created_at":"2025-08-24T10:30:00Z","updated_at":"2025-08-24T16:00:00Z","external_user":null,"creator_user_id":"user_2dHFtVWx2n56w6HkM0000000000","workspace_id":"0df9e665-d932-5740-b2c7-b52af166bc11"}}"#
private let fileJSON = #"{"id":"cfile_b3V0L3JlcG9ydC5jc3Y","object":"container.file","container_id":"sess_abc123","bytes":123,"created_at":1755640000,"path":"out/report.csv","source":"assistant"}"#

private struct Expected: Sendable {
    let method: String
    let target: String
    var body: String? = nil
    var status = 200
    let response: String
}
private actor Recording: HTTPTransport {
    var expected: [Expected]
    var calls = 0
    init(_ expected: [Expected]) { self.expected = expected }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        calls += 1
        guard !expected.isEmpty else { XCTFail("unexpected request"); throw TransportError.invalidURL }
        let fixture = expected.removeFirst()
        XCTAssertEqual(request.method, fixture.method)
        XCTAssertEqual(request.url.absoluteString, "https://openrouter.ai/api/v1" + fixture.target)
        XCTAssertEqual(request.body.map { String(decoding: $0, as: UTF8.self) }, fixture.body)
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "authorization" }.map(\.value), ["Bearer test-management-token"])
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "accept" }.map(\.value), ["application/json"])
        XCTAssertEqual(request.headers.filter { $0.name.lowercased() == "content-type" }.map(\.value), fixture.body == nil ? [] : ["application/json"])
        return HTTPResponse(status: fixture.status, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(fixture.response.utf8))
    }
}

@MainActor
final class OpenRouterNativeTests: XCTestCase {
    func testFiveActualOperationsAndTypedDeclaredFailure() async throws {
        let recording = Recording([
            Expected(method: "GET", target: "/credits", response: #"{"data":{"total_credits":100.50000000000000001,"total_usage":25.75}}"#),
            Expected(method: "GET", target: "/credits", status: 401, response: #"{"error":{"code":401,"message":"Missing Authentication header"}}"#),
            Expected(method: "POST", target: "/keys", body: #"{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}"#, status: 201, response: createJSON),
            Expected(method: "PATCH", target: "/keys/" + keyHash, body: #"{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}"#, response: updateJSON),
            Expected(method: "GET", target: "/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A", response: fileJSON),
            Expected(method: "GET", target: "/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA", response: #"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#),
        ])
        let client = Client(credentials: Credentials(apiKey: "test-management-token"), transport: recording)
        switch try await client.getCredits(GetCreditsInput()) {
        case .status200(let response):
            XCTAssertEqual(response.data.data.totalCredits.raw, "100.50000000000000001")
            XCTAssertEqual(response.data.data.totalUsage.raw, "25.75")
        }
        do { _ = try await client.getCredits(GetCreditsInput()); XCTFail("401 returned success") }
        catch let error as GetCreditsAPIError {
            switch error {
            case .status401(let response):
                XCTAssertEqual(response.data.error.code.raw, "401")
                XCTAssertEqual(response.data.error.message, "Missing Authentication header")
                XCTAssertEqual(response.data.userId, .missing)
            default: XCTFail("wrong error status")
            }
        }
        var create = CreateBody(name: "Native Test Key")
        create.limit = .value(try JsonNumber("50.25"))
        create.limitReset = .null
        switch try await client.createKeys(CreateKeysInput(body: create)) {
        case .status201(let response):
            XCTAssertEqual(response.data.key, "sk-or-v1-fixture-only")
            XCTAssertEqual(response.data.data.limit.valueIfPresent?.raw, "50.250")
            XCTAssertEqual(response.data.data.updatedAt, .null)
            XCTAssertEqual(response.data.data.externalUser, .null)
            XCTAssertEqual(response.data.data.expiresAt, .missing)
        }
        var update = UpdateBody()
        update.disabled = .value(true)
        update.limit = .value(try JsonNumber("75.50"))
        update.limitReset = .null
        update.name = .value("Updated Native Key")
        switch try await client.updateKeys(UpdateKeysInput(hash: keyHash, body: update)) {
        case .status200(let response):
            XCTAssertEqual(response.data.data.limitRemaining.valueIfPresent?.raw, "49.5")
            XCTAssertEqual(response.data.data.limit.valueIfPresent?.raw, "75.50")
        }
        switch try await client.getContainerFile(GetContainerFileInput(containerId: "sess_abc123", fileId: "cfile_a/b 雪!'()*")) {
        case .status200(let response):
            XCTAssertEqual(response.data.bytes.raw, "123")
            XCTAssertEqual(response.data.createdAt.raw, "1755640000")
            XCTAssertEqual(response.data.path, "out/report.csv")
        }
        switch try await client.listContainerFiles(ListContainerFilesInput(containerId: "sess_abc123", limit: .value(2), after: .value("a/b +雪"))) {
        case .status200(let response):
            XCTAssertTrue(response.data.data.isEmpty)
            XCTAssertEqual(response.data.firstId, .null)
            XCTAssertFalse(response.data.hasMore)
        }
        let calls = await recording.calls
        XCTAssertEqual(calls, 6)
    }

    func testMutableCreateInputIsValidatedBeforeNetwork() async throws {
        let recording = Recording([])
        let client = Client(credentials: Credentials(apiKey: "test-management-token"), transport: recording)
        var body = CreateBody(name: "valid")
        body.name = ""
        do { _ = try await client.createKeys(CreateKeysInput(body: body)); XCTFail("invalid name sent") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
        let calls = await recording.calls
        XCTAssertEqual(calls, 0)
        XCTAssertThrowsError(try CreateBody.codec.decode(Data(#"{"name":"x","limit_reset":"yearly"}"#.utf8)))
        XCTAssertThrowsError(try CreateBody.codec.decode(Data(#"{"name":"x","external":{}}"#.utf8)))
        let presence = try CreateBody.codec.decode(Data(#"{"name":"x","limit":null,"expires_at":null,"external":{"user":"u","api_key":"12345678901234567890123456789012"}}"#.utf8))
        XCTAssertEqual(presence.limit, .null)
        XCTAssertEqual(presence.expiresAt, .null)
        XCTAssertEqual(presence.external.valueIfPresent?.user, "u")
        XCTAssertEqual(presence.external.valueIfPresent?.apiKey.valueIfPresent, "12345678901234567890123456789012")
    }

    func testURLSessionCallsAllFiveActualOperations() async throws {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_HTTP_BASE"])
        let client = Client(credentials: Credentials(apiKey: "test-management-token"), options: ClientOptions(serverURL: base))
        switch try await client.getCredits(GetCreditsInput()) {
        case .status200(let response): XCTAssertEqual(response.data.data.totalCredits.raw, "100.50000000000000001")
        }
        do { _ = try await client.getCredits(GetCreditsInput()); XCTFail("401 accepted") }
        catch let error as GetCreditsAPIError {
            switch error {
            case .status401(let response): XCTAssertEqual(response.data.error.code.raw, "401")
            default: XCTFail("wrong typed API error")
            }
        }
        var create = CreateBody(name: "Native Test Key")
        create.limit = .value(try JsonNumber("50.25"))
        create.limitReset = .null
        switch try await client.createKeys(CreateKeysInput(body: create)) {
        case .status201(let response): XCTAssertEqual(response.data.data.limit.valueIfPresent?.raw, "50.250")
        }
        var update = UpdateBody()
        update.disabled = .value(true)
        update.limit = .value(try JsonNumber("75.50"))
        update.limitReset = .null
        update.name = .value("Updated Native Key")
        switch try await client.updateKeys(UpdateKeysInput(hash: keyHash, body: update)) {
        case .status200(let response): XCTAssertEqual(response.data.data.limit.valueIfPresent?.raw, "75.50")
        }
        switch try await client.getContainerFile(GetContainerFileInput(containerId: "sess_abc123", fileId: "cfile_a/b 雪!'()*")) {
        case .status200(let response): XCTAssertEqual(response.data.path, "out/report.csv")
        }
        switch try await client.listContainerFiles(ListContainerFilesInput(containerId: "sess_abc123", limit: .value(2), after: .value("a/b +雪"))) {
        case .status200(let response): XCTAssertEqual(response.data.firstId, .null)
        }
    }
}
