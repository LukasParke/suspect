import Darwin
import Foundation
import XCTest
import GeneratedSDK

private actor Recording: HTTPTransport {
    private var requests: [HTTPRequest] = []
    func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        return HTTPResponse(status: 200, headers: [HTTPHeader("Content-Type", "application/json")], body: Data(#"{"ok":true}"#.utf8))
    }
    func calls() -> Int { requests.count }
    func lastURL() -> String? { requests.last?.url.absoluteString }
}

private struct Usage {
    let peakBytes: Int64
    let cpuSeconds: Double
    static func read() throws -> Usage {
        var usage = rusage()
        guard getrusage(RUSAGE_SELF, &usage) == 0 else {
            throw NSError(domain: "review-rusage", code: Int(errno))
        }
        // Darwin reports ru_maxrss in bytes. Each consumer starts in a fresh
        // XCTest process; this measures actual process allocation pressure.
        return Usage(peakBytes: Int64(usage.ru_maxrss),
                     cpuSeconds: Double(usage.ru_utime.tv_sec) + Double(usage.ru_utime.tv_usec) / 1_000_000)
    }
}

@MainActor
final class QueryExpansionTests: XCTestCase {
    func testAmplificationIsBoundedAtThePublicClientBoundary() async throws {
        let recording = Recording()
        let client = Client(credentials: Credentials(apiKey: "review-fixture-token"), transport: recording)
        let key = String(repeating: "_", count: 4096)
        let values = (0..<10_000).map { "v\($0)" }

        // Warm lazy program/codec/Foundation state before measuring the call.
        _ = try await client.expand(ExpandInput(value: .value(["warm"]), id: "x"))
        let before = try Usage.read()
        do {
            _ = try await client.expand(ExpandInput(value: .value(values), id: "x"))
            XCTFail("10,000 repeated 4-KiB query keys exceeded the generated ceiling")
        } catch let error as SDKError {
            XCTAssertEqual(error.kind, .requestRepresentation)
            XCTAssertEqual(error.json?.kind, .resourceLimit)
        }
        let after = try Usage.read()
        let growth = max(0, after.peakBytes - before.peakBytes)
        print("SWIFT_REVIEW_QUERY peak_growth_bytes=\(growth) cpu_seconds=\(after.cpuSeconds - before.cpuSeconds) expanded_key_bytes=\(values.count * key.utf8.count) ceiling=8388608")
        // The old eager map retained ~39 MiB of repeated keys alone. This
        // permits 3x the 8-MiB ceiling for allocator/native-conversion overhead
        // while rejecting that expansion, without a timing-based assertion.
        XCTAssertLessThan(growth, 24 * 1024 * 1024, "query expansion allocated before its resource guard")
        let callsAfterLimit = await recording.calls()
        XCTAssertEqual(callsAfterLimit, 1, "oversized input reached the transport")

        // This array fits alone; the earlier parameter consumes its remaining
        // budget. It must fail before constructing the ~8-MiB second parameter.
        let sharedBefore = try Usage.read()
        do {
            _ = try await client.expand(ExpandInput(lead: .value(String(repeating: "x", count: 32_768)), value: .value(Array(repeating: "v", count: 2040)), id: "x"))
            XCTFail("query budget restarted at a parameter boundary")
        } catch let error as SDKError {
            XCTAssertEqual(error.kind, .requestRepresentation)
            XCTAssertEqual(error.json?.kind, .resourceLimit)
        }
        let sharedAfter = try Usage.read()
        let sharedGrowth = max(0, sharedAfter.peakBytes - sharedBefore.peakBytes)
        print("SWIFT_REVIEW_QUERY remaining_budget_peak_growth_bytes=\(sharedGrowth)")
        XCTAssertLessThan(sharedGrowth, 6 * 1024 * 1024, "remaining budget was checked after second-parameter expansion")
        let callsAfterSharedLimit = await recording.calls()
        XCTAssertEqual(callsAfterSharedLimit, 1)

        // Comma-joined values must also be sized before percent expansion.
        // This valid ~3-MiB JSON array would expand to ~9 MiB of URI text.
        do {
            let spaces = String(repeating: " ", count: 1_048_576)
            _ = try await client.expand(ExpandInput(labels: .value([spaces, spaces, spaces]), id: "x"))
            XCTFail("comma-joined percent expansion escaped the byte ceiling")
        } catch let error as SDKError {
            XCTAssertEqual(error.kind, .requestRepresentation)
            XCTAssertEqual(error.json?.kind, .resourceLimit)
        }
        let callsAfterJoinedLimit = await recording.calls()
        XCTAssertEqual(callsAfterJoinedLimit, 1)

        // Existing scalar/array wire behavior is independently specified.
        _ = try await client.expand(ExpandInput(lead: .value("a+雪"), value: .value(["x", "!"]), labels: .value(["a,b", "c"]), amount: .value(try JsonNumber("1.2300e+2")), enabled: .value(false), id: "a/b"))
        let actual = await recording.lastURL()
        XCTAssertEqual(actual, "https://example.test/api/v1/expand/a%2Fb?lead=a%2B%E9%9B%AA&\(key)=x&\(key)=%21&labels=a%2Cb,c&amount=1.2300e%2B2&enabled=false")
        _ = try await client.expand(ExpandInput(value: .value([]), id: "empty"))
        let empty = await recording.lastURL()
        XCTAssertEqual(empty, "https://example.test/api/v1/expand/empty")
    }
}
