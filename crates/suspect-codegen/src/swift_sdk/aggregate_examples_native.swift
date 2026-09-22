import Foundation
import XCTest
import GeneratedSDK

private actor Capture: HTTPTransport {
    private var requests: [HTTPRequest] = []
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        return HTTPResponse(status: 204, headers: [], body: Data())
    }
    func only() throws -> HTTPRequest {
        XCTAssertEqual(requests.count, 1)
        return try XCTUnwrap(requests.first)
    }
}

@MainActor
final class AggregateExampleTests: XCTestCase {
    private func segments(_ request: HTTPRequest) throws -> [String] {
        let contentType = try XCTUnwrap(request.headers.first { $0.name.lowercased() == "content-type" }?.value)
        let boundary = try XCTUnwrap(contentType.components(separatedBy: "boundary=").last)
        let body = String(decoding: try XCTUnwrap(request.body), as: UTF8.self)
        return body.components(separatedBy: "--" + boundary).filter { $0.hasPrefix("\r\n") }
    }
    func testDeclaredFormPreservesEmptyArraysExtrasAndOptionalMissing() async throws {
        let capture = Capture()
        _ = try await Examples.declaredForm(client: Client(transport: capture))
        let request = try await capture.only()
        XCTAssertEqual(request.url.absoluteString, "https://api.swift.test/form")
        XCTAssertEqual(String(decoding: try XCTUnwrap(request.body), as: UTF8.self), "id=alpha+beta&note=provided&x=1&x=2")
    }
    func testDeclaredNamedMultipartKeepsRepeatedItemsAndExtraNames() async throws {
        let capture = Capture()
        _ = try await Examples.declaredNamed(client: Client(transport: capture))
        let request = try await capture.only()
        let parts = try segments(request)
        XCTAssertEqual(parts.count, 4)
        XCTAssertTrue(parts[0].contains("name=\"id\"")); XCTAssertTrue(parts[0].hasSuffix("\r\n\r\nA\r\n"))
        XCTAssertTrue(parts[1].contains("name=\"items\"")); XCTAssertTrue(parts[1].hasSuffix("\r\n\r\n1\r\n"))
        XCTAssertTrue(parts[2].contains("name=\"items\"")); XCTAssertTrue(parts[2].hasSuffix("\r\n\r\n2\r\n"))
        XCTAssertTrue(parts[3].contains("name=\"x\"")); XCTAssertTrue(parts[3].hasSuffix("\r\n\r\ntail\r\n"))
        XCTAssertFalse(parts.joined().contains("omitted"))
    }
    func testDeclaredPositionalPrefixAndTailKeepTheirOrderAndValues() async throws {
        let capture = Capture()
        _ = try await Examples.declaredOrdered(client: Client(transport: capture))
        let request = try await capture.only()
        let parts = try segments(request)
        XCTAssertEqual(parts.count, 4)
        for (part, value) in zip(parts, ["7", "declared", "false", "true"]) { XCTAssertTrue(part.hasSuffix("\r\n\r\n" + value + "\r\n"), part) }
    }
    func testDeclaredEmptyPositionalArrayDoesNotSynthesizeAPart() async throws {
        let capture = Capture()
        _ = try await Examples.declaredEmpty(client: Client(transport: capture))
        let request = try await capture.only()
        XCTAssertTrue(try segments(request).isEmpty)
        let contentType = try XCTUnwrap(request.headers.first { $0.name.lowercased() == "content-type" }?.value)
        let boundary = try XCTUnwrap(contentType.components(separatedBy: "boundary=").last)
        XCTAssertEqual(String(decoding: try XCTUnwrap(request.body), as: UTF8.self), "--" + boundary + "--\r\n")
    }
}
