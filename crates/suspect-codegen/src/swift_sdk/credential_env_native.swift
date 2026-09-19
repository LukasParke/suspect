import Foundation
import Darwin
import XCTest
import SwiftEnvSDK

private let bearer = "SUSPECT_SWIFT_ENV_BEARER"
private let header = "SUSPECT_SWIFT_ENV_HEADER"
private let query = "SUSPECT_SWIFT_ENV_QUERY"
private let cookie = "SUSPECT_SWIFT_ENV_COOKIE"

@MainActor
private func environment(_ values: [String: String] = [:]) {
    for name in [bearer, header, query, cookie] {
        if let value = values[name] { XCTAssertEqual(Darwin.setenv(name, value, 1), 0) }
        else { XCTAssertEqual(Darwin.unsetenv(name), 0) }
    }
}

private actor Capture: HTTPTransport {
    private var requests: [HTTPRequest] = []
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        XCTAssertEqual(request.method, "GET")
        XCTAssertTrue(request.url.absoluteString.hasPrefix("https://default.swift.test/api/v1/"))
        let body = request.url.path.hasSuffix("/model") ? #"{"environment":"native-model"}"# : #"{"ok":true}"#
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(body.utf8))
    }
    func all() -> [HTTPRequest] { requests }
    func count() -> Int { requests.count }
}
private func headerValues(_ request: HTTPRequest, _ name: String) -> [String] { request.headers.filter { $0.name.lowercased() == name.lowercased() }.map(\.value) }
private func sendable<T: Sendable>(_ value: T) {}

@MainActor
final class CredentialEnvironmentTests: XCTestCase {
    private func missing(_ operation: () async throws -> Void, forbidden: [String] = []) async {
        do { try await operation(); XCTFail("missing/unusable credentials reached transport") }
        catch let error as SDKError {
            XCTAssertEqual(error.kind, .requestRepresentation)
            XCTAssertNil(error.status); XCTAssertTrue(error.headers.isEmpty); XCTAssertTrue(error.body.isEmpty)
            let description = error.description + (error.json?.description ?? "")
            XCTAssertLessThan(description.utf8.count, 1024)
            for value in forbidden { XCTAssertFalse(description.contains(value)) }
        }
        catch { XCTFail("expected bounded SDKError") }
    }

    func testOmittedInitializerAndFactoryReadBoundVariablesAndUseSourceServer() async throws {
        environment([bearer:"environment-token"])
        let capture = Capture()
        let omitted = Client(transport: capture)
        let factory = Client.fromEnvironment(transport: capture, options: ClientOptions(timeout: 15))
        sendable(omitted); sendable(factory)
        let first = try await omitted.getCurrentKey()
        let second = try await factory.getCurrentKey()
        XCTAssertTrue(first.data.ok); XCTAssertTrue(second.data.ok)
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 2)
        for request in requests {
            XCTAssertEqual(request.url.absoluteString, "https://default.swift.test/api/v1/key")
            XCTAssertEqual(headerValues(request, "Authorization"), ["Bearer environment-token"])
        }
        XCTAssertEqual(requests[1].timeout, 15)
    }

    func testMissingAndEmptyEnvironmentLeaveAnonymousOperationsUsable() async throws {
        environment()
        let capture = Capture(); let client = Client.fromEnvironment(transport: capture)
        _ = try await client.anonymous(); _ = try await client.anonymousChoice()
        await missing { _ = try await client.getCurrentKey() }
        environment([bearer:""])
        let empty = Client(transport: capture)
        await missing { _ = try await empty.getCurrentKey() }
        _ = try await empty.anonymousChoice()
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 3)
        for request in requests { XCTAssertTrue(headerValues(request, "Authorization").isEmpty) }
    }

    func testWholeExplicitCredentialsValueEmptyNilAndMissingMembersWin() async throws {
        environment([bearer:"environment-secret", header:"environment-header"])
        let capture = Capture()
        _ = try await Client(credentials: Credentials(apiKey: "explicit-token"), transport: capture).getCurrentKey()
        for credentials in [Credentials(apiKey: ""), Credentials(apiKey: nil), Credentials()] {
            let client = Client(credentials: credentials, transport: capture)
            await missing({ _ = try await client.getCurrentKey() }, forbidden: ["environment-secret", "environment-header"])
        }
        let partial = Client(credentials: Credentials(headerKey: "explicit-header"), transport: capture)
        await missing { _ = try await partial.bothKeys() }
        _ = try await partial.eitherKey()
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 2)
        XCTAssertEqual(headerValues(requests[0], "Authorization"), ["Bearer explicit-token"])
        XCTAssertEqual(headerValues(requests[1], "X-API-Key"), ["explicit-header"])
        XCTAssertTrue(headerValues(requests[1], "Authorization").isEmpty)
    }

    func testORANDAndExplicitAlternativeSelectionStaySourceDefined() async throws {
        environment([header:"header-only"])
        let capture = Capture(); let client = Client.fromEnvironment(transport: capture)
        _ = try await client.eitherKey()
        _ = try await client.eitherKey(options: RequestOptions(securityAlternative: 1))
        await missing { _ = try await client.eitherKey(options: RequestOptions(securityAlternative: 0)) }
        await missing { _ = try await client.bothKeys() }
        environment([bearer:"both-token", header:"both-header"])
        let both = Client(transport: capture)
        _ = try await both.bothKeys()
        _ = try await both.anonymousChoice(options: RequestOptions(securityAlternative: 1))
        await missing { _ = try await both.eitherKey(options: RequestOptions(securityAlternative: 2)) }
        let requests = await capture.all()
        XCTAssertEqual(requests.count, 4)
        XCTAssertEqual(headerValues(requests[0], "X-API-Key"), ["header-only"])
        XCTAssertEqual(headerValues(requests[2], "Authorization"), ["Bearer both-token"])
        XCTAssertEqual(headerValues(requests[2], "X-API-Key"), ["both-header"])
        XCTAssertTrue(headerValues(requests[3], "Authorization").isEmpty)
    }

    func testAPIKeysKeepHeaderQueryAndCookieAttachment() async throws {
        environment([header:"literal-header", query:"A B+%/雪", cookie:"cookie:value +"])
        let capture = Capture(); let client = Client(transport: capture)
        _ = try await client.eitherKey()
        _ = try await client.queryCredential(); _ = try await client.cookieCredential()
        let requests = await capture.all()
        XCTAssertEqual(headerValues(requests[0], "X-API-Key"), ["literal-header"])
        XCTAssertEqual(requests[1].url.absoluteString, "https://default.swift.test/api/v1/query?access_key=A%20B%2B%25%2F%E9%9B%AA")
        XCTAssertEqual(headerValues(requests[2], "Cookie"), ["session=cookie%3Avalue%20%2B"])
    }

    func testClientCreationSnapshotSurvivesEnvironmentMutationAndTaskHandoff() async throws {
        environment([bearer:"first-token"])
        let firstCapture = Capture(); let first = Client(transport: firstCapture)
        environment([bearer:"second-token"])
        let secondCapture = Capture(); let second = Client.fromEnvironment(transport: secondCapture)
        _ = try await Task.detached { try await first.getCurrentKey() }.value
        _ = try await second.getCurrentKey()
        environment()
        _ = try await first.getCurrentKey(); _ = try await second.getCurrentKey()
        let empty = Client(transport: firstCapture)
        await missing { _ = try await empty.getCurrentKey() }
        let oldRequests = await firstCapture.all(); let newRequests = await secondCapture.all()
        XCTAssertEqual(oldRequests.count, 2); XCTAssertEqual(newRequests.count, 2)
        for request in oldRequests { XCTAssertEqual(headerValues(request, "Authorization"), ["Bearer first-token"]) }
        for request in newRequests { XCTAssertEqual(headerValues(request, "Authorization"), ["Bearer second-token"]) }
    }

    func testUnusableEnvironmentValuesDoNotPoisonAUsableAlternative() async throws {
        environment([bearer:"PRIVATE_ENV_CANARY\ninvalid", header:"usable-header"])
        let capture = Capture(); let client = Client(transport: capture)
        _ = try await client.eitherKey()
        await missing({ _ = try await client.getCurrentKey() }, forbidden: ["PRIVATE_ENV_CANARY", "usable-header"])
        environment([bearer:"é", header:"PRIVATE_HEADER\r\ninvalid", query:"usable-query"])
        let other = Client.fromEnvironment(transport: capture)
        await missing({ _ = try await other.eitherKey() }, forbidden: ["PRIVATE_HEADER"])
        _ = try await other.queryCredential()
        let requests = await capture.all(); XCTAssertEqual(requests.count, 2)
        XCTAssertEqual(headerValues(requests[0], "X-API-Key"), ["usable-header"])
    }

    func testCredentialValueLimitIs8192UTF8Bytes() async throws {
        let capture = Capture()
        environment([bearer:String(repeating: "a", count: 8192), header:String(repeating: "é", count: 4096)])
        let maximum = Client.fromEnvironment(transport: capture)
        _ = try await maximum.getCurrentKey()
        _ = try await maximum.eitherKey(options: RequestOptions(securityAlternative: 1))
        let requests = await capture.all()
        XCTAssertEqual(headerValues(requests[0], "Authorization")[0].utf8.count, 8192 + 7)
        XCTAssertEqual(headerValues(requests[1], "X-API-Key")[0].utf8.count, 8192)
        environment([bearer:String(repeating: "b", count: 8193), header:String(repeating: "é", count: 4096) + "x", query:String(repeating: "q", count: 8193), cookie:String(repeating: "c", count: 8193)])
        let over = Client(transport: capture)
        await missing { _ = try await over.getCurrentKey() }
        await missing { _ = try await over.eitherKey() }
        await missing { _ = try await over.queryCredential() }
        await missing { _ = try await over.cookieCredential() }
        let count = await capture.count(); XCTAssertEqual(count, 2)
    }

    func testHelperAndSourceNamesRemainIndependentlyCallable() async throws {
        environment()
        let capture = Capture(); let client = Client.fromEnvironment(transport: capture)
        let result = try await client.fromEnvironment2()
        XCTAssertEqual(result.data.environment, "native-model")
        let _: (Credentials, any HTTPTransport, ClientOptions) -> Client = Client.init(credentials:transport:options:)
        let _: (any HTTPTransport, ClientOptions) -> Client = Client.fromEnvironment(transport:options:)
    }
}
