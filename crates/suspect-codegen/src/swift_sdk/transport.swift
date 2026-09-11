import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// One HTTP header. Names compare case-insensitively only at the HTTP boundary.
public struct HTTPHeader: Sendable, Equatable {
    public let name: String
    public let value: String
    public init(_ name: String, _ value: String) { self.name = name; self.value = value }
}

/// Complete replayable request from the source-derived operation plan. Custom
/// transports must honor timeout, cancellation, redirect and response ceilings.
public struct HTTPRequest: Sendable {
    public let method: String
    public let url: URL
    public let headers: [HTTPHeader]
    public let body: Data?
    public let timeout: TimeInterval
    public let maxResponseBytes: Int
}

/// Bounded raw response. The client verifies custom transport results again.
public struct HTTPResponse: Sendable {
    public let status: Int
    public let headers: [HTTPHeader]
    public let body: Data
    public init(status: Int, headers: [HTTPHeader], body: Data) {
        self.status = status; self.headers = headers; self.body = body
    }
}

/// Injectable, concurrent async transport. A cancelled call must promptly throw
/// CancellationError; implementations must stop I/O when their task is cancelled.
public protocol HTTPTransport: Sendable {
    func send(_ request: HTTPRequest) async throws -> HTTPResponse
    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse
}

/// Declared response data together with exact status, headers and raw bytes.
public struct APIResponse<Value: Sendable>: Sendable {
    public let status: Int
    public let headers: [HTTPHeader]
    public let data: Value
    public let rawBody: Data
    public let rawBodyTruncated: Bool
    /// Actual wire Content-Type, when present; no media type is sniffed.
    public let contentType: String?
    public let declaredContentType: String?
    public let links: [HTTPLink]
    init(status: Int, headers: [HTTPHeader], data: Value, rawBody: Data, rawBodyTruncated: Bool,
         contentType: String? = nil, declaredContentType: String? = nil, links: [HTTPLink] = []) {
        self.status = status; self.headers = headers; self.data = data; self.rawBody = rawBody
        self.rawBodyTruncated = rawBodyTruncated; self.contentType = contentType
        self.declaredContentType = declaredContentType; self.links = links
    }
}

/// SDK failures are separate from generated per-operation API error enums and
/// native CancellationError. Descriptions never print credentials or body bytes.
public struct SDKError: Error, Sendable, CustomStringConvertible {
    public enum Kind: String, Sendable {
        case configuration, requestValidation, requestRepresentation, responseDecoding
        case unexpectedResponse, responseTooLarge, timeout, transport
    }
    public let kind: Kind
    public let source: SourceLocation
    public let status: Int?
    public let headers: [HTTPHeader]
    public let body: Data
    public let bodyTruncated: Bool
    public let validation: ValidationError?
    public let json: JsonError?
    public var description: String { "\(kind.rawValue) at \(source)\(status.map { " (HTTP \($0))" } ?? "")" }

    init(_ kind: Kind, source: SourceLocation, response: HTTPResponse? = nil,
         captureLimit: Int = 0, validation: ValidationError? = nil, json: JsonError? = nil) {
        self.kind = kind; self.source = source; status = response?.status
        headers = response?.headers ?? []
        body = Data((response?.body ?? Data()).prefix(max(0, captureLimit)))
        bodyTruncated = (response?.body.count ?? 0) > max(0, captureLimit)
        self.validation = validation; self.json = json
    }
}

/// Default transport faults, carrying no request URLs or credential values.
public enum TransportError: Error, Sendable {
    case invalidURL
    case redirectBlocked(status: Int)
    case responseTooLarge(limit: Int)
    case headersTooLarge
    case timedOut
    case network(domain: String, code: Int)
    case streamAlreadyConsumed
    case streamBufferExceeded(limit: Int)
    case exactMethodUnavailable
    case invalidRequestFraming
    case invalidResponseFraming
}

/// Client-wide server selection and finite transport policy. Cleartext overrides
/// require loopback or allowHTTP; declared server URLs retain their scheme.
/// Per-call limits may only lower generated ceilings. Timeouts cover the transfer.
public struct ClientOptions: Sendable {
    public var serverURL: String?
    public var serverIndex: Int
    public var serverVariables: [String: String]
    public var documentURL: String?
    public var securityAlternative: Int?
    public var allowHTTP: Bool
    public var timeout: TimeInterval
    public var maxResponseBytes: Int?
    public init(serverURL: String? = nil, timeout: TimeInterval = 60, maxResponseBytes: Int? = nil,
                serverIndex: Int = 0, serverVariables: [String: String] = [:], documentURL: String? = nil,
                securityAlternative: Int? = nil, allowHTTP: Bool = false) {
        self.serverURL = serverURL; self.timeout = timeout; self.maxResponseBytes = maxResponseBytes
        self.serverIndex = serverIndex; self.serverVariables = serverVariables; self.documentURL = documentURL
        self.securityAlternative = securityAlternative; self.allowHTTP = allowHTTP
    }
}
/// Explicit per-call timeout and a response ceiling no larger than the client ceiling.
public struct RequestOptions: Sendable {
    public var timeout: TimeInterval?
    public var maxResponseBytes: Int?
    public var serverURL: String?
    public var serverIndex: Int?
    public var serverVariables: [String: String]?
    public var documentURL: String?
    public var securityAlternative: Int?
    /// A concrete request Content-Type; it must select the same typed body case.
    public var contentType: String?
    public var multipartBoundary: String?
    public init(timeout: TimeInterval? = nil, maxResponseBytes: Int? = nil,
                serverURL: String? = nil, serverIndex: Int? = nil, serverVariables: [String: String]? = nil,
                documentURL: String? = nil, securityAlternative: Int? = nil,
                contentType: String? = nil, multipartBoundary: String? = nil) {
        self.timeout = timeout; self.maxResponseBytes = maxResponseBytes
        self.serverURL = serverURL; self.serverIndex = serverIndex; self.serverVariables = serverVariables
        self.documentURL = documentURL; self.securityAlternative = securityAlternative
        self.contentType = contentType; self.multipartBoundary = multipartBoundary
    }
}

/// Foundation URLSession baseline: ephemeral sessions, no cookie/cache/credential
/// storage, no redirects or automatic retries, capped delegate accumulation.
/// Each transfer owns its session and invalidates it on completion/cancellation.
public struct URLSessionTransport: HTTPTransport {
    public init() {}
    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        try Task.checkCancellation()
        if !HTTPBuild.urlSessionPreservesMethod(request) { return try await HTTPExactMethodTransport.send(request) }
        let transfer = URLSessionTransfer(limit: request.maxResponseBytes)
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                transfer.start(request, continuation: continuation)
            }
        } onCancel: {
            transfer.cancel()
        }
    }
}

// Buffered transfer state. All mutable state is protected by lock; no user
// closure or continuation is invoked under it.
// Cancellation before continuation/task installation is recorded and replayed.
private final class URLSessionTransfer: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private let limit: Int
    private var continuation: CheckedContinuation<HTTPResponse, any Error>?
    private var session: URLSession?
    private var task: URLSessionDataTask?
    private var response: (Int, [HTTPHeader])?
    private var buffer = Data()
    private var finished = false
    private var cancelled = false
    private var method = ""

    init(limit: Int) { self.limit = limit }
    func start(_ request: HTTPRequest, continuation: CheckedContinuation<HTTPResponse, any Error>) {
        guard limit > 0, request.timeout.isFinite, request.timeout > 0 else {
            continuation.resume(throwing: TransportError.invalidURL); return
        }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = request.timeout
        configuration.timeoutIntervalForResource = request.timeout
        configuration.connectionProxyDictionary = [:]
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        var native = URLRequest(url: request.url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: request.timeout)
        native.httpMethod = request.method; native.httpBody = request.body
        native.httpShouldHandleCookies = false
        for header in request.headers { native.setValue(header.value, forHTTPHeaderField: header.name) }
        let task = session.dataTask(with: native)
        lock.lock()
        if cancelled || finished {
            lock.unlock(); session.invalidateAndCancel()
            continuation.resume(throwing: CancellationError()); return
        }
        self.continuation = continuation; self.session = session; self.task = task; method = request.method
        lock.unlock()
        task.resume()
    }
    func cancel() {
        lock.lock(); cancelled = true; lock.unlock()
        finish(.failure(CancellationError()))
    }
    private func finish(_ result: Result<HTTPResponse, any Error>) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        finished = true
        let continuation = self.continuation; self.continuation = nil
        let session = self.session; self.session = nil
        self.task = nil
        buffer = Data()
        lock.unlock()
        session?.invalidateAndCancel()
        continuation?.resume(with: result)
    }
    func urlSession(_ session: URLSession, task: URLSessionTask,
                    willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest,
                    completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
        finish(.failure(TransportError.redirectBlocked(status: response.statusCode)))
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask,
                    didReceive response: URLResponse, completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse else {
            completionHandler(.cancel); finish(.failure(TransportError.invalidURL)); return
        }
        lock.lock(); let method = self.method; lock.unlock()
        if !HTTPBuild.forbidden(method: method, status: http.statusCode) && response.expectedContentLength > Int64(limit) {
            completionHandler(.cancel); finish(.failure(TransportError.responseTooLarge(limit: limit))); return
        }
        let headers = http.allHeaderFields.map { HTTPHeader(String(describing: $0.key), String(describing: $0.value)) }.sorted { $0.name < $1.name }
        do { try HTTPBuild.checkHeaders(headers) }
        catch { completionHandler(.cancel); finish(.failure(TransportError.headersTooLarge)); return }
        lock.lock()
        let stopped = finished
        if !stopped { self.response = (http.statusCode, headers) }
        lock.unlock()
        completionHandler(stopped ? .cancel : .allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        guard !finished else { lock.unlock(); return }
        if data.count > limit - buffer.count {
            lock.unlock(); finish(.failure(TransportError.responseTooLarge(limit: limit))); return
        }
        buffer.append(data)
        lock.unlock()
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        if let error {
            let native = error as NSError
            let failure: any Error
            if native.domain == NSURLErrorDomain && native.code == NSURLErrorCancelled { failure = CancellationError() }
            else if native.domain == NSURLErrorDomain && native.code == NSURLErrorTimedOut { failure = TransportError.timedOut }
            else { failure = TransportError.network(domain: native.domain, code: native.code) }
            finish(.failure(failure)); return
        }
        lock.lock()
        let response = self.response.map { HTTPResponse(status: $0.0, headers: $0.1, body: buffer) }
        lock.unlock()
        if let response { finish(.success(response)) }
        else { finish(.failure(TransportError.invalidURL)) }
    }
}

enum HTTPBuild {
    static func urlSessionPreservesMethod(_ request: HTTPRequest) -> Bool {
        var native = URLRequest(url: request.url); native.httpMethod = request.method
        return native.httpMethod?.utf8.elementsEqual(request.method.utf8) ?? false
    }
    private static let hex = Array("0123456789ABCDEF".utf8)
    private static func unreserved(_ byte: UInt8) -> Bool {
        (65...90).contains(byte) || (97...122).contains(byte) || (48...57).contains(byte)
            || byte == 45 || byte == 46 || byte == 95 || byte == 126
    }
    static func component(_ value: String) -> String {
        var bytes: [UInt8] = []
        appendComponent(value, to: &bytes)
        return String(decoding: bytes, as: UTF8.self)
    }
    private static func appendComponent(_ value: String, to bytes: inout [UInt8]) {
        for byte in value.utf8 {
            if unreserved(byte) { bytes.append(byte) }
            else { bytes.append(contentsOf: [37, hex[Int(byte >> 4)], hex[Int(byte & 15)]]) }
        }
    }
    // Count before allocation, stopping as soon as the remaining byte budget
    // cannot hold the encoded component. Never construct a 3x temporary first.
    private static func componentSize(_ value: String, limit: Int) throws -> Int {
        guard limit >= 0, value.utf8.count <= limit else { throw queryLimitError() }
        var size = 0
        for byte in value.utf8 {
            let count = unreserved(byte) ? 1 : 3
            guard count <= limit - size else { throw queryLimitError() }
            size += count
        }
        return size
    }
    private static func queryLimitError() -> JsonError {
        JsonError(.resourceLimit, "assembled query byte limit exceeded")
    }
    static func scalar(_ value: JsonValue) throws -> String {
        switch value {
        case .string(let s): return s
        case .number(let n): return n.raw
        case .bool(let b): return b ? "true" : "false"
        default: throw JsonError(.representation, "parameter is not a non-null scalar")
        }
    }
    /// Remaining query capacity after the complete path and server prefix.
    /// No query may consume the byte reserved for its leading question mark.
    static func queryBudget(server: String, path: String, limit: Int) throws -> Int {
        let baseBytes = server.trimmingSuffixSlash.utf8.count
        guard limit > 0, baseBytes <= limit, path.utf8.count <= limit - baseBytes else {
            throw JsonError(.resourceLimit, "assembled URL byte limit exceeded")
        }
        return max(0, limit - baseBytes - path.utf8.count - 1)
    }
    static func appendQuery(_ value: JsonValue, name: String, array: Bool, explode: Bool, required: Bool,
                            to query: inout [String], bytes: inout Int, limit: Int) throws {
        let values: [JsonValue]
        if array {
            guard case .array(let elements) = value else { throw JsonError(.representation, "query value is not an array") }
            if elements.isEmpty {
                if required { throw JsonError(.representation, "a required empty query array has no wire representation") }
                return
            }
            values = elements
        } else { values = [value] }
        guard bytes >= 0, bytes < limit else { throw queryLimitError() }
        let remaining = limit - bytes
        let keySize = try componentSize(name, limit: remaining - 1) + 1 // '='
        if !array || explode {
            let separators = query.isEmpty ? values.count - 1 : values.count
            // Reject multiplicative key expansion before creating even its
            // first pair. Division avoids overflow for large declared counts.
            guard separators <= remaining, keySize <= (remaining - separators) / values.count else {
                throw queryLimitError()
            }
            var key: [UInt8] = []
            key.reserveCapacity(keySize)
            appendComponent(name, to: &key)
            key.append(61)
            for value in values {
                let separator = query.isEmpty ? 0 : 1
                guard separator <= limit - bytes, keySize <= limit - bytes - separator else { throw queryLimitError() }
                let text = try scalar(value)
                let size = try componentSize(text, limit: limit - bytes - separator - keySize)
                // The complete allocation has now been proved to fit the
                // shared remaining budget, including this pair's repeated key.
                var pair: [UInt8] = []
                pair.reserveCapacity(keySize + size)
                pair.append(contentsOf: key)
                appendComponent(text, to: &pair)
                bytes += separator + keySize + size
                query.append(String(decoding: pair, as: UTF8.self))
            }
        } else {
            let separator = query.isEmpty ? 0 : 1
            let commas = values.count - 1
            guard separator <= remaining, keySize <= remaining - separator,
                  commas <= remaining - separator - keySize else { throw queryLimitError() }
            var available = remaining - separator - keySize - commas
            // One comma-joined pair: count all encoded values without retaining
            // another array of encoded strings, then allocate exactly once.
            for value in values {
                available -= try componentSize(scalar(value), limit: available)
            }
            let pairSize = remaining - separator - available
            var pair: [UInt8] = []
            pair.reserveCapacity(pairSize)
            appendComponent(name, to: &pair)
            pair.append(61)
            for (index, value) in values.enumerated() {
                if index > 0 { pair.append(44) }
                appendComponent(try scalar(value), to: &pair)
            }
            bytes += separator + pairSize
            query.append(String(decoding: pair, as: UTF8.self))
        }
    }
    static func checkHeaders(_ headers: [HTTPHeader]) throws {
        var size = 0
        guard headers.count <= 256 else { throw TransportError.headersTooLarge }
        for header in headers {
            guard HTTPWire.token(header.name),
                  !header.value.unicodeScalars.contains(where: { $0.value < 32 && $0.value != 9 || $0.value == 127 }) else { throw TransportError.headersTooLarge }
            let count = header.name.utf8.count + header.value.utf8.count
            guard count <= 65_536 - size else { throw TransportError.headersTooLarge }
            size += count
        }
    }
}
extension String {
    var trimmingSuffixSlash: String {
        var value = self
        while value.hasSuffix("/") { value.removeLast() }
        return value
    }
}
