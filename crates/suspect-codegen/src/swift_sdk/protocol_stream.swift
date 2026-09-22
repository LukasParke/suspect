import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif

/// A response whose body is pulled incrementally. close() is idempotent.
public struct HTTPStreamResponse: Sendable {
    public let status: Int
    public let headers: [HTTPHeader]
    public let body: HTTPByteStream
    public init(status: Int, headers: [HTTPHeader], body: HTTPByteStream) { self.status = status; self.headers = headers; self.body = body }
    public func close() { body.close() }
    func collect(limit: Int) async throws -> HTTPResponse {
        defer { close() }
        var result = Data()
        for try await chunk in body {
            guard chunk.count <= limit - result.count else { throw TransportError.responseTooLarge(limit: limit) }
            result.append(chunk)
        }
        return HTTPResponse(status: status, headers: headers, body: result)
    }
}

// Lifetime tokens are independent of sequence values. A for-await iterator's
// destruction closes I/O even when the caller keeps the response/sequence alive.
// Mutable state is lock-protected; external closures always run outside the lock.
final class HTTPStreamLease: @unchecked Sendable {
    private let lock = NSLock()
    private var action: (@Sendable () -> Void)?
    init(_ action: @escaping @Sendable () -> Void) { self.action = action }
    func close() {
        lock.lock(); let action = self.action; self.action = nil; lock.unlock()
        action?()
    }
    deinit { close() }
}
final class HTTPByteState: @unchecked Sendable {
    private let lock = NSLock()
    private let pull: @Sendable () async throws -> Data?
    private let lifetime: HTTPStreamLease
    private var claimed = false
    private var reading = false
    init(next: @escaping @Sendable () async throws -> Data?, cancel: @escaping @Sendable () -> Void) {
        pull = next; lifetime = HTTPStreamLease(cancel)
    }
    func claim() -> Bool { lock.lock(); defer { lock.unlock() }; if claimed { return false }; claimed = true; return true }
    private func begin() throws {
        lock.lock(); defer { lock.unlock() }
        guard !reading else { throw TransportError.streamAlreadyConsumed }; reading = true
    }
    private func end() { lock.lock(); reading = false; lock.unlock() }
    func next() async throws -> Data? {
        try begin(); defer { end() }
        return try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                let result = try await pull()
                try Task.checkCancellation()
                if result == nil { close() }
                return result
            } catch { close(); if Task.isCancelled { throw CancellationError() }; throw error }
        } onCancel: { self.close() }
    }
    func close() { lifetime.close() }
}

/// Single-consumer, pull-based transport bytes. Custom transports supply bounded
/// chunks and a synchronous cancellation hook. No unbounded AsyncStream queue.
public struct HTTPByteStream: AsyncSequence, Sendable {
    public typealias Element = Data
    private let state: HTTPByteState
    public init(next: @escaping @Sendable () async throws -> Data?, cancel: @escaping @Sendable () -> Void) {
        state = HTTPByteState(next: next, cancel: cancel)
    }
    public struct AsyncIterator: AsyncIteratorProtocol, Sendable {
        private let state: HTTPByteState
        private let valid: Bool
        private let lifetime: HTTPStreamLease
        fileprivate init(_ state: HTTPByteState) {
            self.state = state; valid = state.claim(); lifetime = HTTPStreamLease { state.close() }
        }
        public func next() async throws -> Data? {
            guard valid else { throw TransportError.streamAlreadyConsumed }
            return try await state.next()
        }
    }
    public func makeAsyncIterator() -> AsyncIterator { AsyncIterator(state) }
    public func close() { state.close() }
}
private actor HTTPByteOnce {
    var bytes: Data?
    init(_ bytes: Data) { self.bytes = bytes }
    func next() -> Data? { defer { bytes = nil }; return bytes }
}
public extension HTTPTransport {
    /// Compatibility adapter for finite custom transports. Override to expose
    /// real transport chunks. The default remains bounded by maxResponseBytes.
    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        let response = try await send(request)
        guard response.body.count <= request.maxResponseBytes else { throw TransportError.responseTooLarge(limit: request.maxResponseBytes) }
        let once = HTTPByteOnce(response.body)
        return HTTPStreamResponse(status: response.status, headers: response.headers,
            body: HTTPByteStream(next: { await once.next() }, cancel: {}))
    }
}

enum HTTPStreamFraming: Sendable { case serverSentEvents, jsonLines }
/// A source-typed stream of parsed SSE envelopes or JSON-lines values. Iteration
/// validates each item with its actual itemSchema codec. Breaking iteration,
/// cancellation, close(), codec failure and exhaustion all release the transfer.
public struct HTTPEventStream<Value: Sendable>: AsyncSequence, Sendable {
    public typealias Element = Value
    private let body: HTTPByteStream
    private let codec: ModelCodec<Value>
    private let framing: HTTPStreamFraming
    private let itemLimit: Int
    private let totalLimit: Int
    private let captureLimit: Int
    private let response: HTTPResponse
    init(body: HTTPByteStream, codec: ModelCodec<Value>, framing: HTTPStreamFraming, itemLimit: Int,
         totalLimit: Int, captureLimit: Int, response: HTTPResponse) {
        self.body = body; self.codec = codec; self.framing = framing; self.itemLimit = itemLimit
        self.totalLimit = totalLimit; self.captureLimit = captureLimit; self.response = response
    }
    public struct AsyncIterator: AsyncIteratorProtocol, Sendable {
        private let reader: HTTPEventReader<Value>
        private let lifetime: HTTPStreamLease
        fileprivate init(_ sequence: HTTPEventStream) {
            reader = HTTPEventReader(sequence.body.makeAsyncIterator(), codec: sequence.codec,
                framing: sequence.framing, itemLimit: sequence.itemLimit, totalLimit: sequence.totalLimit,
                captureLimit: sequence.captureLimit, response: sequence.response)
            let body = sequence.body; lifetime = HTTPStreamLease { body.close() }
        }
        public func next() async throws -> Value? {
            do { let value = try await reader.next(); if value == nil { lifetime.close() }; return value }
            catch { lifetime.close(); throw error }
        }
    }
    public func makeAsyncIterator() -> AsyncIterator { AsyncIterator(self) }
    public func close() { body.close() }
}

private actor HTTPEventReader<Value: Sendable> {
    let iterator: HTTPByteStream.AsyncIterator
    let codec: ModelCodec<Value>
    let itemLimit: Int
    let totalLimit: Int
    let captureLimit: Int
    let response: HTTPResponse
    var framer: HTTPFramer
    var chunk = Data()
    var offset = 0
    var total = 0
    var finished = false
    var busy = false
    init(_ iterator: HTTPByteStream.AsyncIterator, codec: ModelCodec<Value>, framing: HTTPStreamFraming,
         itemLimit: Int, totalLimit: Int, captureLimit: Int, response: HTTPResponse) {
        self.iterator = iterator; self.codec = codec; self.itemLimit = itemLimit; self.totalLimit = totalLimit
        self.captureLimit = captureLimit; self.response = response; framer = HTTPFramer(framing: framing, limit: itemLimit)
    }
    func next() async throws -> Value? {
        guard !busy else { throw TransportError.streamAlreadyConsumed }
        busy = true; defer { busy = false }
        if finished { return nil }
        do {
            while true {
                try Task.checkCancellation()
                while offset < chunk.endIndex {
                    let byte = chunk[offset]; offset += 1
                    if let value = try framer.push(byte) { return try codec.decodeValue(value, limits: JsonLimits(maxBytes: itemLimit)) }
                }
                guard let next = try await iterator.next() else {
                    finished = true
                    if let value = try framer.end() { return try codec.decodeValue(value, limits: JsonLimits(maxBytes: itemLimit)) }
                    return nil
                }
                guard next.count <= totalLimit - total else { throw TransportError.responseTooLarge(limit: totalLimit) }
                total += next.count; chunk = next; offset = chunk.startIndex
            }
        } catch {
            finished = true
            if Task.isCancelled || error is CancellationError { throw CancellationError() }
            let captured = HTTPResponse(status: response.status, headers: response.headers, body: framer.errorCapture)
            if let failure = error as? ValidationError { throw SDKError(.responseDecoding, source: codec.source, response: captured, captureLimit: captureLimit, validation: failure) }
            if let failure = error as? JsonError { throw SDKError(.responseDecoding, source: codec.source, response: captured, captureLimit: captureLimit, json: failure) }
            if case TransportError.responseTooLarge = error { throw SDKError(.responseTooLarge, source: codec.source, response: captured, captureLimit: captureLimit) }
            if case TransportError.timedOut = error { throw SDKError(.timeout, source: codec.source, response: captured, captureLimit: captureLimit) }
            throw SDKError(.transport, source: codec.source, response: captured, captureLimit: captureLimit)
        }
    }
}

struct HTTPFramer {
    let framing: HTTPStreamFraming
    let limit: Int
    var line: [UInt8] = []
    var rawCount = 0
    var capture = Data()
    var lastItem = Data()
    var errorCapture: Data { capture.isEmpty ? lastItem : capture }
    var firstLine = true
    var skipLF = false
    var data = ""
    var hasData = false
    var event: String?
    var lastID: String?
    var retry: JsonNumber?
    mutating func push(_ byte: UInt8) throws -> JsonValue? {
        if framing == .serverSentEvents && skipLF { skipLF = false; if byte == 10 { return nil } }
        guard rawCount < limit else { throw JsonError(.resourceLimit, "stream item byte ceiling exceeded") }
        rawCount += 1; capture.append(byte)
        if framing == .jsonLines {
            if byte == 10 {
                let value = try JsonValue.parse(Data(line), limits: JsonLimits(maxBytes: limit))
                lastItem = capture
                line.removeAll(keepingCapacity: false); rawCount = 0; capture.removeAll(keepingCapacity: false)
                return value
            }
            line.append(byte); return nil
        }
        if byte == 10 || byte == 13 {
            if byte == 13 { skipLF = true }
            return try finishLine()
        }
        line.append(byte); return nil
    }
    mutating func finishLine() throws -> JsonValue? {
        if firstLine { firstLine = false; if line.starts(with: [239, 187, 191]) { line.removeFirst(3) } }
        // HTML event-stream uses UTF-8 decoding with replacement. JSON lines
        // instead passes the original bytes to the strict JSON decoder.
        let text = String(decoding: line, as: UTF8.self)
        line.removeAll(keepingCapacity: false)
        if text.isEmpty {
            lastItem = capture
            defer { data = ""; hasData = false; event = nil; retry = nil; rawCount = 0; capture.removeAll(keepingCapacity: false) }
            guard hasData else { return nil }
            var envelope = JsonObject<JsonValue>()
            envelope["data"] = .string(String(data.dropLast()))
            if let event, !event.isEmpty { envelope["event"] = .string(event) }
            if let lastID { envelope["id"] = .string(lastID) }
            if let retry { envelope["retry"] = .number(retry) }
            return .object(envelope)
        }
        if text.hasPrefix(":") { return nil }
        let field: String; var value: String
        if let colon = text.firstIndex(of: ":") {
            field = String(text[..<colon]); value = String(text[text.index(after: colon)...])
            if value.hasPrefix(" ") { value.removeFirst() }
        } else { field = text; value = "" }
        switch field {
        case "data": data += value + "\n"; hasData = true
        case "event": event = value
        case "id": if !value.unicodeScalars.contains(where: { $0.value == 0 }) { lastID = value }
        case "retry":
            if !value.isEmpty, value.utf8.allSatisfy({ (48...57).contains($0) }) {
                let digits = value.drop(while: { $0 == "0" }); retry = try JsonNumber(digits.isEmpty ? "0" : String(digits))
            }
        default: break
        }
        return nil
    }
    mutating func end() throws -> JsonValue? {
        // SSE requires the blank-line dispatch delimiter, including at EOF.
        if framing == .jsonLines && !line.isEmpty { return try JsonValue.parse(Data(line), limits: JsonLimits(maxBytes: limit)) }
        return nil
    }
}

public extension URLSessionTransport {
    func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        try Task.checkCancellation()
        if !HTTPBuild.urlSessionPreservesMethod(request) { return try await HTTPExactMethodTransport.open(request, maxBufferedBytes: maxBufferedBytes) }
        let transfer = HTTPURLSessionStreamTransfer(limit: request.maxResponseBytes, bufferLimit: maxBufferedBytes)
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { transfer.start(request, continuation: $0) }
        } onCancel: { transfer.cancel() }
    }
}

// Delegate state is lock-protected. The session/task, header continuation and
// pending read have a single terminal transition. URLSession callbacks and user
// continuations never run under the lock. Suspension is demand driven; queued
// callback races are handled by the independent finite buffer ceiling.
private final class HTTPURLSessionStreamTransfer: NSObject, URLSessionDataDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private let limit: Int
    private let bufferLimit: Int
    private var session: URLSession?
    private var task: URLSessionDataTask?
    private var head: CheckedContinuation<HTTPStreamResponse, any Error>?
    private var read: CheckedContinuation<Data?, any Error>?
    private var buffer = Data()
    private var total = 0
    private var terminal: Result<Void, any Error>?
    private var suspended = false
    private var method = ""
    init(limit: Int, bufferLimit: Int) { self.limit = limit; self.bufferLimit = bufferLimit }
    func start(_ request: HTTPRequest, continuation: CheckedContinuation<HTTPStreamResponse, any Error>) {
        guard limit > 0, bufferLimit > 0, request.timeout.isFinite, request.timeout > 0 else { continuation.resume(throwing: TransportError.invalidURL); return }
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false; configuration.httpCookieStorage = nil; configuration.urlCredentialStorage = nil
        configuration.urlCache = nil; configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = request.timeout; configuration.timeoutIntervalForResource = request.timeout
        configuration.connectionProxyDictionary = [:]
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        var native = URLRequest(url: request.url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: request.timeout)
        native.httpMethod = request.method; native.httpBody = request.body; native.httpShouldHandleCookies = false
        for header in request.headers { native.setValue(header.value, forHTTPHeaderField: header.name) }
        let task = session.dataTask(with: native)
        lock.lock()
        if terminal != nil { lock.unlock(); session.invalidateAndCancel(); continuation.resume(throwing: CancellationError()); return }
        self.session = session; self.task = task; head = continuation; method = request.method
        lock.unlock(); task.resume()
    }
    func cancel() { finish(.failure(CancellationError()), discard: true) }
    func next() async throws -> Data? {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { install($0) }
        } onCancel: { self.cancel() }
    }
    private func install(_ continuation: CheckedContinuation<Data?, any Error>) {
        lock.lock()
        if !buffer.isEmpty { let bytes = buffer; buffer = Data(); lock.unlock(); continuation.resume(returning: bytes); return }
        if let terminal { lock.unlock(); continuation.resume(with: terminal.map { nil }); return }
        guard read == nil else { lock.unlock(); continuation.resume(throwing: TransportError.streamAlreadyConsumed); return }
        read = continuation
        let resume = suspended ? task : nil; suspended = false
        lock.unlock(); resume?.resume()
    }
    private func finish(_ result: Result<Void, any Error>, discard: Bool = false) {
        lock.lock()
        guard terminal == nil else { if discard { buffer = Data() }; lock.unlock(); return }
        terminal = result
        let head = self.head; self.head = nil; let read = self.read; self.read = nil
        let session = self.session; self.session = nil; self.task = nil
        if discard { buffer = Data() }
        lock.unlock(); session?.invalidateAndCancel()
        head?.resume(throwing: result.failure ?? TransportError.invalidURL)
        read?.resume(with: result.map { nil })
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil); finish(.failure(TransportError.redirectBlocked(status: response.statusCode)), discard: true)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let response = response as? HTTPURLResponse else { completionHandler(.cancel); finish(.failure(TransportError.invalidURL)); return }
        lock.lock(); let method = self.method; lock.unlock()
        if !HTTPBuild.forbidden(method: method, status: response.statusCode) && response.expectedContentLength > Int64(limit) {
            completionHandler(.cancel); finish(.failure(TransportError.responseTooLarge(limit: limit))); return
        }
        let headers = response.allHeaderFields.map { HTTPHeader(String(describing: $0.key), String(describing: $0.value)) }.sorted { $0.name < $1.name }
        do { try HTTPBuild.checkHeaders(headers) } catch { completionHandler(.cancel); finish(.failure(TransportError.headersTooLarge)); return }
        lock.lock(); let continuation = head; head = nil; let stopped = terminal != nil; lock.unlock()
        if stopped { completionHandler(.cancel); return }
        let body = HTTPByteStream(next: { try await self.next() }, cancel: { self.cancel() })
        continuation?.resume(returning: HTTPStreamResponse(status: response.statusCode, headers: headers, body: body))
        completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        guard terminal == nil else { lock.unlock(); return }
        guard data.count <= limit - total else { lock.unlock(); finish(.failure(TransportError.responseTooLarge(limit: limit)), discard: true); return }
        total += data.count
        let continuation = read; read = nil
        if continuation == nil {
            guard data.count <= bufferLimit - buffer.count else { lock.unlock(); finish(.failure(TransportError.streamBufferExceeded(limit: bufferLimit)), discard: true); return }
            buffer.append(data)
        }
        if !suspended { suspended = true; dataTask.suspend() }
        lock.unlock(); continuation?.resume(returning: data)
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        if let error {
            let native = error as NSError
            if native.domain == NSURLErrorDomain && native.code == NSURLErrorCancelled { finish(.failure(CancellationError())) }
            else if native.domain == NSURLErrorDomain && native.code == NSURLErrorTimedOut { finish(.failure(TransportError.timedOut)) }
            else { finish(.failure(TransportError.network(domain: native.domain, code: native.code))) }
        } else { finish(.success(())) }
    }
}
private extension Result where Success == Void, Failure == any Error {
    var failure: (any Error)? { if case .failure(let error) = self { return error }; return nil }
}
