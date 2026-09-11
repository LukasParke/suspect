import Foundation
import XCTest
import GeneratedSDK

private let octets = Data([0, 255, 128, 13, 10, 65])
private actor Recording: HTTPTransport {
    var calls: [HTTPRequest] = []
    var gone = false
    func setGone() { gone = true }
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        calls.append(request)
        if request.url.path.hasSuffix("/credits/coinbase") {
            XCTAssertEqual(request.method, "POST"); XCTAssertNil(request.body)
            XCTAssertFalse(request.headers.contains { $0.name.lowercased() == "authorization" })
            if gone { return HTTPResponse(status: 410, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(#"{"error":{"code":410,"message":"gone"}}"#.utf8)) }
            return HTTPResponse(status: 200, headers: [], body: octets)
        }
        XCTAssertEqual(request.method, "GET")
        XCTAssertEqual(request.headers.first { $0.name.lowercased() == "authorization" }?.value, "Bearer fixture-token")
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/octet-stream")], body: octets)
    }
    func recorded() -> [HTTPRequest] { calls }
}

@MainActor
final class OpenRouterProtocolTests: XCTestCase {
    func testActualNewOperationsUseNativeBytesAndAnonymousConvenience() async throws {
        let recording = Recording()
        let client = Client(credentials: Credentials(apiKey: "fixture-token"), transport: recording)
        let file = try await client.downloadFileContent(DownloadFileContentInput(fileId: "or_file_1", workspaceId: .value("work &雪")))
        XCTAssertEqual(file.data, octets); XCTAssertEqual(file.status, 200)
        let container = try await client.downloadContainerFileContent(DownloadContainerFileContentInput(containerId: "sess/a", fileId: "cfile_1"))
        XCTAssertEqual(container.data, octets)
        let anonymous = try await client.createCoinbaseCharge()
        XCTAssertEqual(anonymous.data, octets, "an unspecified 200 body is bytes, not void")
        let requests = await recording.recorded()
        XCTAssertEqual(requests[0].url.absoluteString, "https://openrouter.ai/api/v1/files/or_file_1/content?workspace_id=work%20%26%E9%9B%AA")
        XCTAssertEqual(requests[1].url.absoluteString, "https://openrouter.ai/api/v1/containers/sess%2Fa/files/cfile_1/content")
        await recording.setGone()
        do { _ = try await client.createCoinbaseCharge(); XCTFail("declared failure returned success") }
        catch CreateCoinbaseChargeAPIError.status410(let response) { XCTAssertEqual(response.status, 410); XCTAssertEqual(response.data.error.message, "gone") }
        do { _ = try await client.downloadFileContent(DownloadFileContentInput(fileId: "bad/slash")); XCTFail("original file-id pattern ignored") }
        catch let error as SDKError { XCTAssertEqual(error.kind, .requestValidation) }
    }
    func testActualNewOperationsOverURLSession() async throws {
        let base = try XCTUnwrap(ProcessInfo.processInfo.environment["SUSPECT_SWIFT_NEW_OPENROUTER_BASE"])
        let client = Client(credentials: Credentials(apiKey: "fixture-token"), options: ClientOptions(serverURL: base + "/api/v1", timeout: 3))
        let file = try await client.downloadFileContent(DownloadFileContentInput(fileId: "or_file_1")); XCTAssertEqual(file.data, octets)
        let container = try await client.downloadContainerFileContent(DownloadContainerFileContentInput(containerId: "sess/a", fileId: "cfile_1")); XCTAssertEqual(container.data, octets)
        let charge = try await client.createCoinbaseCharge(); XCTAssertEqual(charge.data, octets)
    }
}
