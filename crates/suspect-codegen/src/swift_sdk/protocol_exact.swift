import Foundation
#if canImport(Network)
import Network
import Security
#endif

/// URLRequest canonicalizes several *custom*, case-sensitive HTTP method tokens.
/// This internal HTTP/1.1 path is used only when URLSession would change a token.
/// It retains the same deadline, trust, cancellation, redirect and byte policies.
enum HTTPExactMethodTransport {
    static func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        let response = try await open(request, maxBufferedBytes: min(65_536, request.maxResponseBytes))
        return try await response.collect(limit: request.maxResponseBytes)
    }
    static func open(_ request: HTTPRequest, maxBufferedBytes: Int) async throws -> HTTPStreamResponse {
        #if canImport(Network)
        let connection = try HTTPExactConnection(request: request)
        return try await withTaskCancellationHandler {
            do {
                try Task.checkCancellation()
                try await connection.start()
                try await connection.send(HTTPExactRequestHead.make(request))
                if let body = request.body, !body.isEmpty { try await connection.send(body) }
                let reader = HTTPExactResponseReader(connection: connection, request: request, bufferLimit: maxBufferedBytes)
                let head = try await reader.head()
                try Task.checkCancellation()
                let bytes = HTTPByteStream(next: { try await reader.next() }, cancel: { connection.cancel() })
                return HTTPStreamResponse(status: head.status, headers: head.headers, body: bytes)
            } catch { connection.cancel(); if Task.isCancelled { throw CancellationError() }; throw error }
        } onCancel: { connection.cancel() }
        #else
        throw TransportError.exactMethodUnavailable
        #endif
    }
}

private enum HTTPExactRequestHead {
    static func make(_ request: HTTPRequest) throws -> Data {
        guard HTTPWire.token(request.method), let components = URLComponents(url: request.url, resolvingAgainstBaseURL: false),
              let host = components.percentEncodedHost else { throw TransportError.invalidURL }
        var target = components.percentEncodedPath.isEmpty ? "/" : components.percentEncodedPath
        if let query = components.percentEncodedQuery { target += "?" + query }
        var headers = request.headers
        guard !headers.contains(where: { $0.name.lowercased() == "transfer-encoding" }) else { throw TransportError.invalidRequestFraming }
        let lengths = headers.filter { $0.name.lowercased() == "content-length" }
        guard lengths.count <= 1, lengths.first.map({ $0.value == String(request.body?.count ?? 0) }) ?? true else { throw TransportError.invalidRequestFraming }
        if !headers.contains(where: { $0.name.lowercased() == "host" }) {
            headers.append(HTTPHeader("Host", host + (components.port.map { ":\($0)" } ?? "")))
        }
        if !headers.contains(where: { $0.name.lowercased() == "connection" }) { headers.append(HTTPHeader("Connection", "close")) }
        if lengths.isEmpty { headers.append(HTTPHeader("Content-Length", String(request.body?.count ?? 0))) }
        if !headers.contains(where: { $0.name.lowercased() == "accept-encoding" }) { headers.append(HTTPHeader("Accept-Encoding", "identity")) }
        try HTTPBuild.checkHeaders(headers)
        // URL and header domains were independently bounded before transport.
        var output = HTTPWireBuffer(limit: target.utf8.count + request.method.utf8.count + 65_568)
        try output.append(request.method + " " + target + " HTTP/1.1\r\n")
        for header in headers { try output.append(header.name + ": " + header.value + "\r\n") }
        try output.append("\r\n")
        return output.data
    }
}

#if canImport(Network)
// NWConnection is used from its serial queue; continuation installation and the
// single terminal transition are lock-protected. Nothing resumes under the lock.
private final class HTTPExactConnection: @unchecked Sendable {
    private let lock = NSLock()
    private let connection: NWConnection
    private let queue = DispatchQueue(label: "suspect.http.exact-method")
    private let timeout: TimeInterval
    private var deadline: DispatchWorkItem?
    private var failure: (any Error)?
    private var ready: CheckedContinuation<Void, any Error>?
    private var receiving: CheckedContinuation<(Data, Bool), any Error>?
    private var writing: CheckedContinuation<Void, any Error>?

    init(request: HTTPRequest) throws {
        guard request.timeout.isFinite, request.timeout > 0, request.timeout <= 86_400,
              request.maxResponseBytes > 0, let host = request.url.host,
              let port = NWEndpoint.Port(rawValue: UInt16(exactly: request.url.port ?? (request.url.scheme == "https" ? 443 : 80)) ?? 0),
              port.rawValue > 0, ["http", "https"].contains(request.url.scheme ?? "") else { throw TransportError.invalidURL }
        let hostname = host.trimmingCharacters(in: CharacterSet(charactersIn: "[]"))
        let tls: NWProtocolTLS.Options?
        if request.url.scheme == "https" {
            let options = NWProtocolTLS.Options()
            sec_protocol_options_set_tls_server_name(options.securityProtocolOptions, hostname)
            sec_protocol_options_add_tls_application_protocol(options.securityProtocolOptions, "http/1.1")
            sec_protocol_options_set_peer_authentication_required(options.securityProtocolOptions, true)
            tls = options // system trust evaluation; no custom trust bypass
        } else { tls = nil }
        connection = NWConnection(host: NWEndpoint.Host(hostname), port: port, using: NWParameters(tls: tls, tcp: NWProtocolTCP.Options()))
        timeout = request.timeout
    }
    func start() async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            lock.lock()
            if let failure { lock.unlock(); continuation.resume(throwing: failure); return }
            ready = continuation
            let timer = DispatchWorkItem { [weak self] in self?.finish(TransportError.timedOut) }
            deadline = timer
            lock.unlock()
            connection.stateUpdateHandler = { [weak self] state in
                guard let self else { return }
                switch state {
                case .ready: self.becameReady()
                case .failed(let error), .waiting(let error): self.finish(Self.networkError(error))
                case .cancelled: self.finish(CancellationError())
                default: break
                }
            }
            queue.asyncAfter(deadline: .now() + timeout, execute: timer)
            connection.start(queue: queue)
        }
    }
    private func becameReady() {
        lock.lock(); let continuation = ready; ready = nil; lock.unlock()
        continuation?.resume()
    }
    func send(_ data: Data) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            lock.lock()
            if let failure { lock.unlock(); continuation.resume(throwing: failure); return }
            guard writing == nil else { lock.unlock(); continuation.resume(throwing: TransportError.invalidRequestFraming); return }
            writing = continuation; lock.unlock()
            connection.send(content: data, completion: .contentProcessed { [weak self] error in
                guard let self else { return }
                if let error { self.finish(Self.networkError(error)); return }
                self.lock.lock(); let continuation = self.writing; self.writing = nil; self.lock.unlock()
                continuation?.resume()
            })
        }
    }
    func receive(limit: Int) async throws -> (Data, Bool) {
        try await withCheckedThrowingContinuation { continuation in
            lock.lock()
            if let failure { lock.unlock(); continuation.resume(throwing: failure); return }
            guard receiving == nil else { lock.unlock(); continuation.resume(throwing: TransportError.streamAlreadyConsumed); return }
            receiving = continuation; lock.unlock()
            connection.receive(minimumIncompleteLength: 1, maximumLength: max(1, limit)) { [weak self] data, _, done, error in
                guard let self else { return }
                if let error { self.finish(Self.networkError(error)); return }
                self.lock.lock(); let continuation = self.receiving; self.receiving = nil; self.lock.unlock()
                continuation?.resume(returning: (data ?? Data(), done))
            }
        }
    }
    func cancel() { finish(CancellationError()) }
    private func finish(_ error: any Error) {
        lock.lock()
        guard failure == nil else { lock.unlock(); return }
        failure = error
        let ready = self.ready; self.ready = nil
        let receiving = self.receiving; self.receiving = nil
        let writing = self.writing; self.writing = nil
        let deadline = self.deadline; self.deadline = nil
        lock.unlock()
        deadline?.cancel(); connection.cancel()
        ready?.resume(throwing: error); receiving?.resume(throwing: error); writing?.resume(throwing: error)
    }
    private static func networkError(_ error: NWError) -> TransportError {
        switch error {
        case .posix: return .network(domain: "POSIX", code: (error as NSError).code)
        case .dns(let code): return .network(domain: "DNS", code: Int(code))
        case .tls(let code): return .network(domain: "TLS", code: Int(code))
        default: return .network(domain: "Network", code: -1)
        }
    }
}

private actor HTTPExactResponseReader {
    private enum Framing { case length(Int), chunked, untilEOF, none }
    let connection: HTTPExactConnection
    let request: HTTPRequest
    let bufferLimit: Int
    var buffer = Data()
    var eof = false
    var finished = false
    private var framing: Framing = .none
    var total = 0
    var chunkRemaining = 0
    var chunkEnd = false
    init(connection: HTTPExactConnection, request: HTTPRequest, bufferLimit: Int) {
        self.connection = connection; self.request = request; self.bufferLimit = max(1, min(bufferLimit, 65_536))
    }
    func head() async throws -> HTTPResponse {
        var headBytes = 0
        for _ in 0..<16 {
            let block = try await through(Data([13, 10, 13, 10]), limit: 65_536 - headBytes)
            headBytes += block.count
            let text = try HTTPWire.text(block)
            var lines = text.components(separatedBy: "\r\n")
            guard let first = lines.first else { throw TransportError.invalidResponseFraming }
            let statusLine = first.split(separator: " ", maxSplits: 2, omittingEmptySubsequences: false)
            guard statusLine.count >= 2, ["HTTP/1.1", "HTTP/1.0"].contains(statusLine[0]), statusLine[1].utf8.count == 3,
                  statusLine[1].utf8.allSatisfy({ (48...57).contains($0) }), let status = Int(statusLine[1]), (100...599).contains(status) else { throw TransportError.invalidResponseFraming }
            lines.removeFirst(); lines.removeLast(2)
            let headers = try lines.map(Self.header)
            try HTTPBuild.checkHeaders(headers)
            if (100..<200).contains(status) && status != 101 { continue }
            if [301, 302, 303, 307, 308].contains(status) && headers.contains(where: { $0.name.lowercased() == "location" }) { throw TransportError.redirectBlocked(status: status) }
            if HTTPBuild.forbidden(method: request.method, status: status) { framing = .none; buffer = Data(); return HTTPResponse(status: status, headers: headers, body: Data()) }
            if headers.contains(where: { $0.name.lowercased() == "content-encoding" && $0.value.lowercased() != "identity" }) { throw TransportError.invalidResponseFraming }
            let lengths = headers.filter { $0.name.lowercased() == "content-length" }
            let transfers = headers.filter { $0.name.lowercased() == "transfer-encoding" }
            guard lengths.count <= 1, transfers.count <= 1, lengths.isEmpty || transfers.isEmpty else { throw TransportError.invalidResponseFraming }
            if let transfer = transfers.first {
                guard transfer.value.lowercased() == "chunked" else { throw TransportError.invalidResponseFraming }; framing = .chunked
            } else if let length = lengths.first {
                guard !length.value.isEmpty, length.value.utf8.allSatisfy({ (48...57).contains($0) }) else { throw TransportError.invalidResponseFraming }
                guard let count = Int(length.value), count <= request.maxResponseBytes else { throw TransportError.responseTooLarge(limit: request.maxResponseBytes) }
                framing = .length(count)
            } else { framing = .untilEOF }
            return HTTPResponse(status: status, headers: headers, body: Data())
        }
        throw TransportError.headersTooLarge
    }
    func next() async throws -> Data? {
        try Task.checkCancellation()
        if finished { return nil }
        switch framing {
        case .none: finished = true; return nil
        case .length(let remaining):
            if remaining == 0 { finished = true; return nil }
            let data = try await take(maximum: min(remaining, bufferLimit))
            guard !data.isEmpty else { throw TransportError.invalidResponseFraming }
            framing = .length(remaining - data.count)
            return try counted(data)
        case .untilEOF:
            let data = try await take(maximum: bufferLimit)
            if data.isEmpty && eof { finished = true; return nil }
            return try counted(data)
        case .chunked:
            if chunkEnd {
                guard try await through(Data([13, 10]), limit: 2) == Data([13, 10]) else { throw TransportError.invalidResponseFraming }
                chunkEnd = false
            }
            if chunkRemaining == 0 {
                let line = try await through(Data([13, 10]), limit: 8192)
                let text = String(decoding: line.dropLast(2), as: UTF8.self)
                let pieces = try HTTPWire.splitQuoted(text, delimiter: ";")
                guard let size = pieces.first, !size.isEmpty, size.utf8.allSatisfy({ HTTPWire.hex($0) != nil }) else { throw TransportError.invalidResponseFraming }
                for ext in pieces.dropFirst() {
                    let pair = ext.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                    guard HTTPWire.token(String(pair[0])) else { throw TransportError.invalidResponseFraming }
                    if pair.count == 2 { _ = try HTTPWire.unquote(String(pair[1])) }
                }
                guard let size = Int(size, radix: 16), size <= request.maxResponseBytes - total else { throw TransportError.responseTooLarge(limit: request.maxResponseBytes) }
                if size == 0 {
                    var bytes = 0; var trailers: [HTTPHeader] = []
                    while true {
                        let line = try await through(Data([13, 10]), limit: 65_536 - bytes); bytes += line.count
                        if line.count == 2 { break }
                        let header = try Self.header(HTTPWire.text(Data(line.dropLast(2))))
                        guard !["content-length", "transfer-encoding", "content-type", "authorization", "host"].contains(header.name.lowercased()) else { throw TransportError.invalidResponseFraming }
                        trailers.append(header); try HTTPBuild.checkHeaders(trailers)
                    }
                    finished = true; return nil
                }
                chunkRemaining = size
            }
            let data = try await take(maximum: min(chunkRemaining, bufferLimit))
            guard !data.isEmpty else { throw TransportError.invalidResponseFraming }
            chunkRemaining -= data.count; chunkEnd = chunkRemaining == 0
            return try counted(data)
        }
    }
    private func counted(_ data: Data) throws -> Data {
        guard data.count <= request.maxResponseBytes - total else { throw TransportError.responseTooLarge(limit: request.maxResponseBytes) }
        total += data.count; return data
    }
    private func take(maximum: Int) async throws -> Data {
        while buffer.isEmpty && !eof { try await more() }
        let count = min(maximum, buffer.count)
        let data = Data(buffer.prefix(count)); buffer = Data(buffer.dropFirst(count)); return data
    }
    private func through(_ delimiter: Data, limit: Int) async throws -> Data {
        while true {
            if let end = buffer.range(of: delimiter) {
                let count = end.upperBound
                guard count <= limit else { throw TransportError.headersTooLarge }
                let data = Data(buffer.prefix(count)); buffer = Data(buffer.dropFirst(count)); return data
            }
            guard buffer.count < limit else { throw TransportError.headersTooLarge }
            guard !eof else { throw TransportError.invalidResponseFraming }
            try await more()
        }
    }
    private func more() async throws {
        let (data, done) = try await connection.receive(limit: bufferLimit)
        buffer.append(data); eof = done
    }
    private static func header(_ text: String) throws -> HTTPHeader {
        guard let colon = text.firstIndex(of: ":") else { throw TransportError.invalidResponseFraming }
        let name = String(text[..<colon]); let value = text[text.index(after: colon)...].trimmingCharacters(in: .whitespaces)
        guard HTTPWire.token(name) else { throw TransportError.invalidResponseFraming }
        let header = HTTPHeader(name, value); try HTTPBuild.checkHeaders([header]); return header
    }
}
#endif
