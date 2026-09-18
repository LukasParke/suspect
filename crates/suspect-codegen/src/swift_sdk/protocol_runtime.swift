import Foundation

/// HTTP explicitly forbids a response body (HEAD, 1xx, 204 or 304).
public struct HTTPNoContent: Sendable, Equatable { public init() {} }

/// Original HTTP declaration, terminal definition and reference-object hops.
public struct HTTPProvenance: Sendable, Equatable {
    public let useSite: SourceLocation
    public let terminal: SourceLocation
    public let references: [SourceLocation]
    public let useSiteResource: HTTPResourceContext?
    public let terminalResource: HTTPResourceContext?
    public let referenceResources: [HTTPResourceContext?]
}
/// Logical reference metadata is separate from physical source ownership.
/// scopeAddress identifies the registered enclosing scope, not a fabricated
/// address for each unregistered keyword child. These fields perform no I/O.
public struct HTTPResourceContext: Sendable, Equatable {
    public enum Kind: Sendable, Equatable { case document, openAPIDocument, schema }
    public let source: SourceLocation
    public let resource: SourceLocation
    public let kind: Kind
    public let canonicalURI: String
    public let baseURI: String
    public let baseSource: SourceLocation?
    public let scopeAddress: String
    public let schemaRoot: SourceLocation?
    public let aliases: [String]
}
/// Location bases for API URLs, distinct from a logical reference-resource base.
public enum HTTPURLBase: Sendable, Equatable { case serverDocument, effectiveServer }
/// Metadata with its own original source, rather than a nearby model's source.
public struct HTTPLocated<Value: Sendable>: Sendable {
    public let value: Value
    public let source: SourceLocation
}

/// Structured media type. Wildcards occur only in declarations, never on the wire.
public struct HTTPMediaType: Sendable, Equatable {
    public let declared: String
    public let type: String
    public let subtype: String
    public let parameters: [String: String]
    init(declared: String, type: String, subtype: String, parameters: [String: String]) {
        self.declared = declared; self.type = type; self.subtype = subtype; self.parameters = parameters
    }
    public init(_ contentType: String) throws {
        guard contentType.utf8.count <= 65_536,
              !contentType.unicodeScalars.contains(where: { $0.value < 32 && $0.value != 9 || $0.value == 127 }) else {
            throw JsonError(.representation, "invalid Content-Type")
        }
        let pieces = try HTTPWire.splitQuoted(contentType, delimiter: ";")
        let essence = pieces[0].trimmingCharacters(in: .whitespaces).split(separator: "/", omittingEmptySubsequences: false)
        guard essence.count == 2, essence.allSatisfy({ HTTPWire.token(String($0)) && !$0.contains("*") }) else {
            throw JsonError(.representation, "Content-Type requires a concrete type/subtype")
        }
        var parameters: [String: String] = [:]
        for piece in pieces.dropFirst() {
            let pair = piece.trimmingCharacters(in: .whitespaces).split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard pair.count == 2, HTTPWire.token(String(pair[0])) else { throw JsonError(.representation, "invalid media parameter") }
            let name = pair[0].lowercased()
            guard parameters[name] == nil else { throw JsonError(.representation, "duplicate media parameter") }
            parameters[name] = try HTTPWire.unquote(String(pair[1]))
        }
        self.init(declared: contentType, type: essence[0].lowercased(), subtype: essence[1].lowercased(), parameters: parameters)
    }
    var rank: Int { (type == "*" ? 0 : subtype == "*" ? 1 : 2) * 100_000 + parameters.count }
    func matches(_ actual: HTTPMediaType) -> Bool {
        (type == "*" || type == actual.type) && (subtype == "*" || subtype == actual.subtype)
            && parameters.allSatisfy { key, value in
                guard let other = actual.parameters[key] else { return false }
                return key == "charset" ? value.lowercased() == other.lowercased() : value.utf8.elementsEqual(other.utf8)
            }
    }
    static func select(_ actual: String, from declarations: [HTTPMediaType]) throws -> Int {
        let media = try HTTPMediaType(actual)
        guard let selected = declarations.indices.filter({ declarations[$0].matches(media) }).max(by: { declarations[$0].rank < declarations[$1].rank }) else {
            throw JsonError(.representation, "Content-Type does not match a declared representation")
        }
        return selected
    }
    static func contentType(_ headers: [HTTPHeader]) throws -> String {
        let values = headers.filter { $0.name.lowercased() == "content-type" }
        guard values.count == 1 else { throw JsonError(.representation, "one Content-Type is required for declared content") }
        return values[0].value
    }
    static func utf8(_ actual: String) throws {
        let media = try HTTPMediaType(actual)
        guard media.parameters["charset"].map({ $0.lowercased() == "utf-8" }) ?? true else {
            throw JsonError(.representation, "declared text/framing requires UTF-8")
        }
    }
}

/// Literal OpenAPI server variable. Values are validated, never URI-template escaped.
public struct HTTPServerVariable: Sendable {
    public let name: String
    public let source: HTTPProvenance
    public let defaultValue: HTTPLocated<String>
    public let values: [HTTPLocated<String>]?
    public let description: HTTPLocated<String>?
}
/// One explicit server choice, in source order. Local-file documents need documentURL for relative servers.
public struct HTTPServer: Sendable {
    public let source: HTTPProvenance?
    public let defaultFrom: SourceLocation?
    public let documentBase: SourceLocation
    public let urlBase: HTTPURLBase
    public let template: String
    public let name: HTTPLocated<String>?
    public let description: HTTPLocated<String>?
    public let variables: [HTTPServerVariable]
    public func resolve(documentURL: String? = nil, variables overrides: [String: String] = [:], maxBytes: Int = 8 * 1024 * 1024) throws -> URL {
        guard overrides.keys.allSatisfy({ key in variables.contains { $0.name.utf8.elementsEqual(key.utf8) } }) else {
            throw JsonError(.representation, "unknown server variable override")
        }
        var expanded = HTTPWireBuffer(limit: maxBytes)
        var rest = template[...]
        while let open = rest.firstIndex(of: "{") {
            try expanded.append(String(rest[..<open]))
            let start = rest.index(after: open)
            guard let close = rest[start...].firstIndex(of: "}"),
                  let variable = variables.first(where: { $0.name.utf8.elementsEqual(rest[start..<close].utf8) }) else {
                throw JsonError(.representation, "invalid server template")
            }
            let value = overrides.first(where: { $0.key.utf8.elementsEqual(variable.name.utf8) })?.value ?? variable.defaultValue.value
            if let allowed = variable.values, !allowed.contains(where: { $0.value.utf8.elementsEqual(value.utf8) }) {
                throw JsonError(.representation, "server override is not a declared enum value")
            }
            try expanded.append(value); rest = rest[rest.index(after: close)...]
        }
        try expanded.append(String(rest))
        let value = expanded.string
        guard !value.isEmpty, !value.contains(where: { "?#\\{}".contains($0) }),
              value.utf8.allSatisfy({ $0 > 32 && $0 < 127 }), HTTPWire.percentTriples(value),
              let components = URLComponents(string: value), components.string == value else { throw JsonError(.representation, "invalid expanded server URL") }
        let anchor = documentURL ?? documentBase.document
        let absolute: URL
        if components.scheme != nil, let url = components.url { absolute = url }
        else {
            guard anchor.utf8.allSatisfy({ $0 > 32 && $0 < 127 }), !anchor.contains("\\"), HTTPWire.percentTriples(anchor),
                  let metadata = URLComponents(string: anchor), metadata.string == anchor, metadata.fragment == nil,
                  let base = metadata.url, ["http", "https"].contains(base.scheme?.lowercased() ?? ""),
                  let resolved = URL(string: value, relativeTo: base)?.absoluteURL else {
                throw JsonError(.representation, "relative server requires an absolute HTTP document URL")
            }
            absolute = resolved
        }
        guard var normalized = URLComponents(url: absolute, resolvingAgainstBaseURL: false) else { throw JsonError(.representation, "invalid resolved server URL") }
        normalized.scheme = normalized.scheme?.lowercased()
        normalized.percentEncodedHost = normalized.percentEncodedHost?.lowercased()
        normalized.percentEncodedPath = Self.literalDotSegments(normalized.percentEncodedPath)
        guard let result = normalized.url else { throw JsonError(.representation, "invalid resolved server URL") }
        try HTTPBuild.serverPolicy(result.absoluteString, maxBytes: maxBytes, allowHTTP: true, encodedDotsAreData: true)
        return result
    }
    // RFC 3986 removes only literal dot segments. Encoded slashes/dots and
    // repeated separators remain data; Foundation does not normalize absolute
    // reference paths, so perform that final step for both reference forms.
    private static func literalDotSegments(_ path: String) -> String {
        if path.isEmpty { return path }
        let pieces = path.split(separator: "/", omittingEmptySubsequences: false)
        var result: [Substring] = []
        for (index, piece) in pieces.enumerated() {
            if piece == "." {
                if index == pieces.count - 1 { result.append("") }
            } else if piece == ".." {
                if result.count > (path.hasPrefix("/") ? 1 : 0) { result.removeLast() }
                if index == pieces.count - 1 { result.append("") }
            } else { result.append(piece) }
        }
        return result.joined(separator: "/")
    }
}

/// Complete caller-supplied Basic credentials; no credential storage is consulted.
public struct HTTPBasicCredential: Sendable {
    public var username: String
    public var password: String
    public init(username: String, password: String) { self.username = username; self.password = password }
}
public enum HTTPPermission: Sendable {
    case scopes([HTTPLocated<String>])
    case roles([HTTPLocated<String>])
}
public struct HTTPOAuthFlow: Sendable {
    public let source: SourceLocation
    public let kind: String
    public let urlBase: HTTPURLBase
    public let authorizationURL: HTTPLocated<String>?
    public let tokenURL: HTTPLocated<String>?
    public let refreshURL: HTTPLocated<String>?
    public let deviceAuthorizationURL: HTTPLocated<String>?
    public let scopes: JsonObject<HTTPLocated<String>>
}
/// Source-backed authorization context. A hook returns the complete Authorization
/// field; the SDK never infers its token type, discovers endpoints or refreshes it.
public struct HTTPCredentialContext: Sendable {
    public enum Kind: Sendable { case bearer, basic, apiKey(location: String, name: String), oauth2, openIdConnect }
    public let source: SourceLocation
    public let scheme: HTTPProvenance
    public let name: String
    public let kind: Kind
    public let permissions: HTTPPermission
    public let description: HTTPLocated<String>?
    public let bearerFormat: HTTPLocated<String>?
    public let flows: [HTTPOAuthFlow]
    public let metadataURL: HTTPLocated<String>?
    public let discoveryURL: HTTPLocated<String>?
    public let urlBase: HTTPURLBase?
    /// Populated for a credential hook invocation after server selection.
    public internal(set) var serverURL: URL? = nil
}
public typealias HTTPAuthorizationProvider = @Sendable (HTTPCredentialContext) async throws -> String
enum HTTPAuthValue: Sendable { case text(String), basic(HTTPBasicCredential), provider(HTTPAuthorizationProvider) }
public enum HTTPSecurity: Sendable {
    case undeclared(SourceLocation)
    case disabled(SourceLocation)
    /// Outer alternatives are OR; each inner list is AND. An empty list is anonymous.
    case alternatives([[HTTPCredentialContext]])
}

public enum HTTPLinkTarget: Sendable {
    case operationID(value: HTTPLocated<String>, operation: SourceLocation)
    case operationReference(value: HTTPLocated<String>, operation: SourceLocation)
}
/// Literal values and runtime-expression text are metadata, never implicit API calls.
public struct HTTPLink: Sendable {
    public let name: String
    public let source: HTTPProvenance
    public let target: HTTPLinkTarget
    public let parameters: JsonObject<HTTPLocated<JsonValue>>
    public let requestBody: HTTPLocated<JsonValue>?
    public let description: HTTPLocated<String>?
    public let server: HTTPServer?
}
public struct HTTPMetadata: Sendable {
    public let source: HTTPProvenance
    public let servers: [HTTPServer]
    public let security: HTTPSecurity
}
struct HTTPResponseRule: Sendable {
    let exact: Int?
    let range: Int?
    let media: [HTTPMediaType]
    let source: SourceLocation
    func rank(_ status: Int) -> Int { exact.map { $0 == status ? 3 : 0 } ?? range.map { $0 == status / 100 ? 2 : 0 } ?? 1 }
    static func select(_ response: HTTPResponse, from rules: [Self], source: SourceLocation, capture: Int) throws -> Int {
        guard let index = rules.indices.filter({ rules[$0].rank(response.status) > 0 }).max(by: { rules[$0].rank(response.status) < rules[$1].rank(response.status) }) else {
            throw SDKError(.unexpectedResponse, source: source, response: response, captureLimit: capture)
        }
        return index
    }
}
struct HTTPBody: Sendable { let bytes: Data; let contentType: String }

extension HTTPBuild {
    static func safePath(_ path: String) throws {
        guard path.hasPrefix("/"), !path.contains(where: { "?#{}\\".contains($0) }), HTTPWire.percentTriples(path),
              !path.split(separator: "/", omittingEmptySubsequences: false).contains(where: {
                  let value = String($0).removingPercentEncoding; return value == "." || value == ".."
              }) else { throw JsonError(.representation, "unsafe expanded path") }
    }
    static func textBody(_ bytes: Data, contentType: String) throws -> String {
        try HTTPMediaType.utf8(contentType); return try HTTPWire.text(bytes)
    }
    static func textValue(_ bytes: Data, contentType: String, scalar: HTTPScalar) throws -> JsonValue {
        try HTTPWire.scalarValue(textBody(bytes, contentType: contentType), type: scalar)
    }
    static func eventStream<T>(_ response: HTTPStreamResponse, codec: ModelCodec<T>, contentType: String,
                               framing: HTTPStreamFraming, itemLimit: Int, totalLimit: Int, captureLimit: Int) throws -> HTTPEventStream<T> {
        try HTTPMediaType.utf8(contentType)
        return HTTPEventStream(body: response.body, codec: codec, framing: framing, itemLimit: itemLimit,
            totalLimit: totalLimit, captureLimit: captureLimit,
            response: HTTPResponse(status: response.status, headers: response.headers, body: Data()))
    }
    static func serverPolicy(_ base: String, maxBytes: Int, allowHTTP: Bool, encodedDotsAreData: Bool = false) throws {
        guard base.utf8.count <= maxBytes, !base.contains("\\"), HTTPWire.percentTriples(base),
              !base.unicodeScalars.contains(where: { $0.value <= 32 || $0.value == 127 }),
              let url = URLComponents(string: base), let host = url.host, !host.isEmpty,
              url.user == nil, url.password == nil, url.query == nil, url.fragment == nil,
              url.scheme?.lowercased() == "https" || url.scheme?.lowercased() == "http" && (allowHTTP || ["127.0.0.1", "localhost", "[::1]", "::1"].contains(host)),
              !url.percentEncodedPath.split(separator: "/", omittingEmptySubsequences: false).contains(where: {
                  let value = encodedDotsAreData ? String($0) : String($0).removingPercentEncoding; return value == "." || value == ".."
              }) else { throw JsonError(.representation, "invalid server URL policy") }
    }
    static func server(_ metadata: HTTPMetadata, options: ClientOptions, request: RequestOptions, maxBytes: Int) throws -> String {
        if let override = request.serverURL ?? options.serverURL {
            guard request.serverIndex == nil, request.serverVariables == nil else { throw JsonError(.representation, "server override cannot also select a template") }
            try serverPolicy(override, maxBytes: maxBytes, allowHTTP: options.allowHTTP)
            return override
        }
        let index = request.serverIndex ?? options.serverIndex
        guard metadata.servers.indices.contains(index) else { throw JsonError(.representation, "server choice is out of range") }
        return try metadata.servers[index].resolve(documentURL: request.documentURL ?? options.documentURL,
            variables: request.serverVariables ?? options.serverVariables, maxBytes: maxBytes).absoluteString
    }
    static func authentication(_ security: HTTPSecurity, selected: Int?, values: [String: HTTPAuthValue], serverURL: String,
                               headers: inout [HTTPHeader], query: inout [String], cookies: inout [String], limit: Int) async throws {
        guard case .alternatives(let alternatives) = security else {
            guard selected == nil else { throw JsonError(.representation, "security is disabled or undeclared") }; return
        }
        let available: (Int) -> Bool = { index in alternatives[index].allSatisfy { values[$0.scheme.useSite.description] != nil } }
        let index: Int
        if let selected {
            guard alternatives.indices.contains(selected), available(selected) else { throw JsonError(.representation, "selected security alternative has missing credentials") }
            index = selected
        } else {
            guard let first = alternatives.indices.first(where: available) else { throw JsonError(.representation, "no security alternative has all required credentials") }
            index = first
        }
        for requirement in alternatives[index] {
            try Task.checkCancellation()
            guard let value = values[requirement.scheme.useSite.description] else { throw JsonError(.representation, "missing credential") }
            switch (requirement.kind, value) {
            case (.bearer, .text(let token)):
                guard !token.isEmpty, token.utf8.count <= 8192, token.utf8.allSatisfy({ (33...126).contains($0) }) else { throw JsonError(.representation, "invalid bearer credential") }
                try attach(HTTPHeader("Authorization", "Bearer " + token), to: &headers)
            case (.basic, .basic(let credential)):
                guard !credential.username.contains(":"), credential.username.utf8.count + credential.password.utf8.count <= 8192,
                      !(credential.username + credential.password).unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }) else { throw JsonError(.representation, "invalid basic credential") }
                try attach(HTTPHeader("Authorization", "Basic " + Data((credential.username + ":" + credential.password).utf8).base64EncodedString()), to: &headers)
            case (.apiKey(let location, let name), .text(let token)):
                guard !token.isEmpty, token.utf8.count <= 8192 else { throw JsonError(.representation, "invalid API key credential") }
                if location == "header" { try attach(HTTPHeader(name, token), to: &headers) }
                else {
                    let key = try HTTPWire.encode(name, encoding: .uriComponent, limit: limit)
                    let pair = key + "=" + (try HTTPWire.encode(token, encoding: .uriComponent, limit: limit))
                    if location == "query" {
                        guard !query.joined(separator: "&").split(separator: "&").contains(where: { $0.split(separator: "=", maxSplits: 1).first == Substring(key) }) else { throw JsonError(.representation, "conflicting query credential attachment") }
                        try appendQueryFragment(pair, to: &query, limit: limit)
                    } else {
                        guard !cookies.joined(separator: "; ").split(separator: ";").contains(where: { $0.trimmingCharacters(in: .whitespaces).hasPrefix(key + "=") }) else { throw JsonError(.representation, "conflicting cookie credential attachment") }
                        try appendCookie(pair, to: &cookies)
                    }
                }
            case (.oauth2, .provider(let provider)), (.openIdConnect, .provider(let provider)):
                var context = requirement
                context.serverURL = URL(string: serverURL)
                let authorization = try await provider(context)
                guard !authorization.isEmpty, authorization.utf8.count <= 8192 else { throw JsonError(.representation, "invalid caller authorization") }
                try attach(HTTPHeader("Authorization", authorization), to: &headers)
            default: throw JsonError(.representation, "credential does not match its declared attachment")
            }
        }
    }
    static func attach(_ header: HTTPHeader, to headers: inout [HTTPHeader]) throws {
        guard headers.count < 256 else { throw JsonError(.resourceLimit, "HTTP header count ceiling exceeded") }
        guard !headers.contains(where: { $0.name.lowercased() == header.name.lowercased() }) else { throw JsonError(.representation, "conflicting header attachment") }
        do { try checkHeaders(headers + [header]) } catch { throw JsonError(.representation, "invalid or oversized header attachment") }
        headers.append(header)
    }
    static func appendCookie(_ value: String, to cookies: inout [String]) throws {
        let used = cookies.reduce(0) { $0 + $1.utf8.count + 2 }
        guard cookies.count < 256, value.utf8.count <= 65_536 - used else { throw JsonError(.resourceLimit, "Cookie header byte ceiling exceeded") }
        cookies.append(value)
    }
    static func appendQueryFragment(_ value: String, to query: inout [String], limit: Int) throws {
        let used = query.reduce(0) { $0 + $1.utf8.count + 1 }
        guard value.utf8.count <= limit - used else { throw JsonError(.resourceLimit, "query credential byte ceiling exceeded") }
        query.append(value)
    }
    /// Runtime-discovered platform version, compact. `operatingSystemVersionString`
    /// is too verbose for the ua/v1 comment; `unknown` degrades without omission.
    static let languageVersion: String = {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        guard version.majorVersion != 0 || version.minorVersion != 0 || version.patchVersion != 0 else { return "unknown" }
        var compact = "\(version.majorVersion).\(version.minorVersion)"
        if version.patchVersion != 0 { compact += ".\(version.patchVersion)" }
        return compact
    }()
    /// ua/v1 application identity: `<name>` or `<name>/<version>` of RFC 9110 tokens.
    static func applicationIdentity(_ value: String) -> Bool {
        guard value.utf8.count <= 128 else { return false }
        let parts = value.split(separator: "/", omittingEmptySubsequences: false)
        return parts.count <= 2 && parts.allSatisfy { HTTPWire.token(String($0)) }
    }
    /// ua/v1 attribution: an explicit non-empty caller User-Agent wins entirely, an
    /// explicit empty value suppresses the header, and the default identifies
    /// suspect as the generator and the SDK package or a caller-supplied
    /// application as the client. An invalid application identity produces no header.
    static func resolveUserAgent(_ options: ClientOptions) -> String? {
        if let userAgent = options.userAgent { return userAgent.isEmpty ? nil : userAgent }
        guard !Attribution.suspectVersion.isEmpty else { return nil }
        var identity = Attribution.sdkName + "/" + Attribution.sdkVersion
        if let applicationId = options.applicationId, !applicationId.isEmpty {
            guard applicationIdentity(applicationId) else { return nil }
            identity = applicationId
        }
        return "suspect/\(Attribution.suspectVersion) \(identity) (\(Attribution.language)/\(languageVersion); openapi/\(Attribution.specVersion))"
    }
    static func makeProtocolRequest(method: String, path: String, query: [String], headers: [HTTPHeader], cookies: [String], body: HTTPBody?,
                                    server: String, options: ClientOptions, request: RequestOptions, maxRequest: Int, maxResponse: Int) throws -> HTTPRequest {
        let timeout = request.timeout ?? options.timeout
        let clientLimit = options.maxResponseBytes ?? maxResponse
        let limit = request.maxResponseBytes ?? clientLimit
        guard timeout.isFinite, timeout > 0, timeout <= 86_400, clientLimit > 0, clientLimit <= maxResponse,
              limit > 0, limit <= clientLimit, (body?.bytes.count ?? 0) <= maxRequest else { throw JsonError(.resourceLimit, "invalid timeout or byte ceiling") }
        var text = HTTPWireBuffer(limit: maxRequest)
        try text.append(server.trimmingSuffixSlash); try text.append(path)
        for (index, pair) in query.enumerated() { try text.append(index == 0 ? "?" : "&"); try text.append(pair) }
        guard let target = URL(string: text.string), target.absoluteString == text.string else { throw JsonError(.representation, "URL cannot preserve the planned wire spelling") }
        var headers = headers
        if !cookies.isEmpty { try attach(HTTPHeader("Cookie", cookies.joined(separator: "; ")), to: &headers) }
        // ua/v1 attribution is applied after declared parameters so an explicit
        // caller-supplied User-Agent header keeps precedence over the default.
        if !headers.contains(where: { $0.name.lowercased() == "user-agent" }) {
            if let userAgent = resolveUserAgent(options) { try attach(HTTPHeader("User-Agent", userAgent), to: &headers) }
        }
        if let body { try attach(HTTPHeader("Content-Type", body.contentType), to: &headers) }
        do { try checkHeaders(headers) } catch { throw JsonError(.resourceLimit, "request header ceiling exceeded") }
        return HTTPRequest(method: method, url: target, headers: headers, body: body?.bytes, timeout: timeout, maxResponseBytes: limit)
    }
    static func forbidden(method: String, status: Int) -> Bool { method == "HEAD" || (100..<200).contains(status) || status == 204 || status == 205 || status == 304 }
}
