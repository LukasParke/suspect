import Foundation

/// A finite in-memory multipart value. File contents are Data, never paths or
/// JSON placeholders. The selected part codec validates value at the call boundary.
public struct HTTPPart<Value: Sendable>: Sendable {
    public var value: Value
    public var filename: String?
    public var contentType: String?
    public init(_ value: Value, filename: String? = nil, contentType: String? = nil) {
        self.value = value; self.filename = filename; self.contentType = contentType
    }
}
struct HTTPPartRule: Sendable {
    let name: String?
    let required: Bool
    let repeated: Bool
    let minItems: UInt64?
    let maxItems: UInt64?
    let contentTypes: [HTTPMediaType]
    let maxBytes: Int
    func checkCount(_ count: Int) throws {
        guard (!required || count > 0), (repeated || count <= 1),
              minItems.map({ UInt64(count) >= $0 }) ?? true,
              maxItems.map({ UInt64(count) <= $0 }) ?? true else { throw JsonError(.representation, "part multiplicity or cardinality violates its declaration") }
    }
    func contentType(_ selected: String?) throws -> String? {
        if contentTypes.isEmpty {
            guard selected == nil else { throw JsonError(.representation, "style-encoded part has no Content-Type selection") }; return nil
        }
        let value = selected ?? contentTypes.first(where: { $0.type != "*" && $0.subtype != "*" })?.declared
        guard let value else { throw JsonError(.representation, "wildcard part requires a concrete Content-Type") }
        _ = try HTTPMediaType.select(value, from: contentTypes)
        if contentTypes.allSatisfy({ $0.type == "text" }) { try HTTPMediaType.utf8(value) }
        return value
    }
}
struct HTTPObjectRules: Sendable {
    let names: [String]
    let required: [String]
    let additional: Bool
    let minProperties: UInt64?
    let maxProperties: UInt64?
    func check(_ present: [String]) throws {
        let keys = Set(present.map(JsonKey.init))
        let declared = Set(names.map(JsonKey.init))
        guard required.allSatisfy({ keys.contains(JsonKey($0)) }), additional || keys.isSubset(of: declared),
              minProperties.map({ UInt64(keys.count) >= $0 }) ?? true,
              maxProperties.map({ UInt64(keys.count) <= $0 }) ?? true else { throw JsonError(.representation, "form/multipart structure violates required, extra-property or cardinality rules") }
    }
}
struct HTTPPositionalRules: Sendable {
    let prefix: Int
    let items: Bool
    let minimum: UInt64?
    let maximum: UInt64?
    func checkCount(_ count: Int) throws {
        guard count <= 100_000, items || count <= prefix,
              minimum.map({ UInt64(count) >= $0 }) ?? true,
              maximum.map({ UInt64(count) <= $0 }) ?? true else {
            throw JsonError(.representation, "positional multipart count violates its prefix/items or cardinality declaration")
        }
    }
    func check(present: [Bool], remaining: Int) throws {
        guard present.count == prefix, remaining <= 100_000 else { throw JsonError(.resourceLimit, "positional multipart item ceiling exceeded") }
        let firstMissing = present.firstIndex(of: false) ?? prefix
        guard present.dropFirst(firstMissing).allSatisfy({ !$0 }), firstMissing == prefix || remaining == 0 else {
            throw JsonError(.representation, "positional multipart cannot skip a prefix position before a later part")
        }
        try checkCount(firstMissing + remaining)
    }
}
struct HTTPRawPart: Sendable {
    let name: String
    let bytes: Data
    let headers: [HTTPHeader]
    let filename: String?
    let contentType: String?
}

enum HTTPParts {
    static func append(_ data: Data, to output: inout Data, limit: Int) throws {
        guard data.count <= limit - output.count else { throw JsonError(.resourceLimit, "assembled body byte ceiling exceeded") }
        output.append(data)
    }
    static func bytes(_ bytes: Data, limit: Int) throws -> Data {
        guard bytes.count <= limit else { throw JsonError(.resourceLimit, "body/part byte ceiling exceeded") }; return bytes
    }
    static func textBytes(_ text: String, limit: Int) throws -> Data {
        guard text.utf8.count <= limit else { throw JsonError(.resourceLimit, "text body/part byte ceiling exceeded") }
        return Data(text.utf8)
    }
    static func quoted(_ value: String) throws -> String {
        guard value.utf8.count <= 8192, !value.unicodeScalars.contains(where: { $0.value < 32 || $0.value == 127 }) else { throw JsonError(.representation, "invalid multipart disposition value") }
        return "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }
    static func boundary(_ selected: String?) throws -> String {
        let value = selected ?? "suspect-" + UUID().uuidString
        guard !value.isEmpty, value.utf8.count <= 70, value.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || "'-_+.".utf8.contains($0) }) else {
            throw JsonError(.representation, "invalid multipart boundary")
        }
        return value
    }
    static func appendPart(name: String, bytes: Data, headers declared: [HTTPHeader], filename: String?, contentType: String?,
                           boundary: String, to output: inout Data, bodyLimit: Int, partLimit: Int) throws {
        guard bytes.count <= partLimit, bytes.count <= bodyLimit - output.count else { throw JsonError(.resourceLimit, "multipart part byte ceiling exceeded") }
        guard bytes.range(of: Data(("--" + boundary).utf8)) == nil else { throw JsonError(.representation, "multipart boundary collides with part data") }
        var headers = declared
        if let header = headers.first(where: { $0.name.lowercased() == "content-disposition" }) {
            let disposition = try parseDisposition(header.value)
            guard disposition.0.utf8.elementsEqual(name.utf8), filename == nil || filename == disposition.1 else { throw JsonError(.representation, "declared disposition conflicts with the named part") }
        } else {
            let disposition = "form-data; name=" + (try quoted(name)) + (try filename.map { "; filename=" + (try quoted($0)) } ?? "")
            try HTTPBuild.attach(HTTPHeader("Content-Disposition", disposition), to: &headers)
        }
        if let contentType { try HTTPBuild.attach(HTTPHeader("Content-Type", contentType), to: &headers) }
        try appendFramedPart(bytes: bytes, headers: headers, boundary: boundary, to: &output, bodyLimit: bodyLimit, partLimit: partLimit)
    }
    static func appendPositionalPart(bytes: Data, headers declared: [HTTPHeader], filename: String?, contentType: String?, formData: Bool,
                                     boundary: String, to output: inout Data, bodyLimit: Int, partLimit: Int) throws {
        var headers = declared
        let dispositions = headers.filter { $0.name.lowercased() == "content-disposition" }
        guard dispositions.count <= 1, !formData || dispositions.count == 1 else { throw JsonError(.representation, "positional form-data requires its declared disposition") }
        if let disposition = dispositions.first {
            let declaredFilename: String?
            if formData { declaredFilename = try parseDisposition(disposition.value).1 }
            else { declaredFilename = try dispositionParameters(disposition.value).1["filename"] }
            guard filename == nil || filename == declaredFilename else { throw JsonError(.representation, "filename conflicts with the declared positional disposition") }
        } else if filename != nil { throw JsonError(.representation, "unnamed positional parts do not infer a Content-Disposition filename") }
        if let contentType { try HTTPBuild.attach(HTTPHeader("Content-Type", contentType), to: &headers) }
        try appendFramedPart(bytes: bytes, headers: headers, boundary: boundary, to: &output, bodyLimit: bodyLimit, partLimit: partLimit)
    }
    private static func appendFramedPart(bytes: Data, headers: [HTTPHeader], boundary: String, to output: inout Data, bodyLimit: Int, partLimit: Int) throws {
        guard bytes.count <= partLimit, bytes.count <= bodyLimit - output.count else { throw JsonError(.resourceLimit, "multipart part byte ceiling exceeded") }
        guard bytes.range(of: Data(("--" + boundary).utf8)) == nil else { throw JsonError(.representation, "multipart boundary collides with part data") }
        do { try HTTPBuild.checkHeaders(headers) } catch { throw JsonError(.representation, "invalid multipart headers") }
        var prefix = HTTPWireBuffer(limit: min(65_536, bodyLimit - output.count - bytes.count))
        try prefix.append("--" + boundary + "\r\n")
        for header in headers { try prefix.append(header.name + ": " + header.value + "\r\n") }
        try prefix.append("\r\n")
        guard prefix.count + 2 <= bodyLimit - output.count - bytes.count else { throw JsonError(.resourceLimit, "multipart framing exceeds body ceiling") }
        try append(prefix.data, to: &output, limit: bodyLimit)
        try append(bytes, to: &output, limit: bodyLimit)
        try append(Data([13, 10]), to: &output, limit: bodyLimit)
    }
    static func finishMultipart(_ output: inout Data, boundary: String, limit: Int) throws {
        try append(Data(("--" + boundary + "--\r\n").utf8), to: &output, limit: limit)
    }
    static func appendForm(_ text: String, name: String, encoding: HTTPPercentEncoding, to output: inout HTTPWireBuffer) throws {
        var count = HTTPWireBuffer(limit: output.limit - output.count, counting: true)
        if output.count > 0 { try count.append("&") }
        try HTTPWire.writeEncoded(name, encoding: encoding, to: &count); try count.append("=")
        try HTTPWire.writeEncoded(text, encoding: encoding, to: &count)
        if output.count > 0 { try output.append("&") }
        try HTTPWire.writeEncoded(name, encoding: encoding, to: &output); try output.append("=")
        try HTTPWire.writeEncoded(text, encoding: encoding, to: &output)
    }
    static func appendStyle(_ text: String, to output: inout HTTPWireBuffer) throws {
        guard text.utf8.count + (output.count == 0 ? 0 : 1) <= output.limit - output.count else { throw JsonError(.resourceLimit, "form expansion exceeds body ceiling") }
        if output.count > 0 { try output.append("&") }; try output.append(text)
    }
    static func parseDisposition(_ value: String) throws -> (String, String?) {
        let (kind, parameters) = try dispositionParameters(value)
        guard kind == "form-data" else { throw JsonError(.representation, "named multipart requires form-data disposition") }
        guard let name = parameters["name"], !name.isEmpty, parameters["filename*"] == nil else { throw JsonError(.representation, "multipart disposition requires a name; filename* is not a form-data parameter") }
        return (name, parameters["filename"])
    }
    private static func dispositionParameters(_ value: String) throws -> (String, [String: String]) {
        let fields = try HTTPWire.splitQuoted(value, delimiter: ";")
        let kind = fields[0].trimmingCharacters(in: .whitespaces).lowercased()
        guard HTTPWire.token(kind) else { throw JsonError(.representation, "invalid disposition token") }
        var parameters: [String: String] = [:]
        for field in fields.dropFirst() {
            let pair = field.trimmingCharacters(in: .whitespaces).split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard pair.count == 2, HTTPWire.token(String(pair[0])), parameters[pair[0].lowercased()] == nil else { throw JsonError(.representation, "invalid disposition parameter") }
            parameters[pair[0].lowercased()] = try HTTPWire.unquote(String(pair[1]))
        }
        return (kind, parameters)
    }
    static func parseMultipart(_ body: Data, contentType: String, partLimit: Int, requireFormDataDisposition: Bool = true) throws -> [HTTPRawPart] {
        let body = Data(body) // normalize Data slice indices, after the outer byte check
        let media = try HTTPMediaType(contentType)
        guard let declared = media.parameters["boundary"] else { throw JsonError(.representation, "multipart Content-Type requires boundary") }
        let boundary = try boundary(declared)
        let marker = Data(("--" + boundary).utf8)
        let separator = Data(("\r\n--" + boundary).utf8)
        let headerEnd = Data([13, 10, 13, 10])
        var index: Int
        if body.starts(with: marker), boundaryLine(body, at: marker.count) != nil { index = marker.count }
        else {
            var search = body.startIndex; var initial: Int?
            while let candidate = body.range(of: separator, in: search..<body.endIndex) {
                if boundaryLine(body, at: candidate.upperBound) != nil { initial = candidate.upperBound; break }
                search = candidate.upperBound
            }
            guard let initial else { throw JsonError(.representation, "missing initial multipart boundary") }
            index = initial // RFC 2046 preamble is commentary, not an extra part
        }
        var parts: [HTTPRawPart] = []
        while true {
            guard let (closing, next) = boundaryLine(body, at: index) else { throw JsonError(.representation, "invalid multipart boundary line") }
            if closing { return parts } // RFC 2046 epilogue is likewise ignored
            index = next
            let head: Range<Int>
            if body.count - index >= 2 && body[index] == 13 && body[index + 1] == 10 { head = index..<(index + 2) }
            else {
                guard let end = body.range(of: headerEnd, in: index..<body.count), end.lowerBound - index <= 65_536 else { throw JsonError(.representation, "missing or oversized multipart headers") }
                head = end
            }
            let text = try HTTPWire.text(body.subdata(in: index..<head.lowerBound))
            let headers = try (text.isEmpty ? [] : text.components(separatedBy: "\r\n")).map { line -> HTTPHeader in
                guard let colon = line.firstIndex(of: ":") else { throw JsonError(.representation, "malformed multipart header") }
                return HTTPHeader(String(line[..<colon]), line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces))
            }
            try HTTPBuild.checkHeaders(headers)
            guard !headers.contains(where: { ["content-transfer-encoding", "transfer-encoding"].contains($0.name.lowercased()) }) else { throw JsonError(.representation, "part transfer encoding is unsupported") }
            let dispositions = headers.filter { $0.name.lowercased() == "content-disposition" }
            guard dispositions.count <= 1, !requireFormDataDisposition || dispositions.count == 1 else { throw JsonError(.representation, "part requires one unambiguous disposition") }
            let name: String; let filename: String?
            if requireFormDataDisposition { (name, filename) = try parseDisposition(dispositions[0].value) }
            else if let disposition = dispositions.first {
                let parameters = try dispositionParameters(disposition.value).1
                name = parameters["name"] ?? ""; filename = parameters["filename"]
            } else { name = ""; filename = nil }
            let types = headers.filter { $0.name.lowercased() == "content-type" }
            guard types.count <= 1 else { throw JsonError(.representation, "duplicate part Content-Type") }
            let start = head.upperBound
            var search = start; var end: Range<Int>?
            while let candidate = body.range(of: separator, in: search..<body.count) {
                let tail = candidate.upperBound
                if boundaryLine(body, at: tail) != nil { end = candidate; break }
                search = candidate.upperBound
            }
            guard let end else { throw JsonError(.representation, "missing final multipart boundary") }
            guard end.lowerBound - start <= partLimit, parts.count < 100_000 else { throw JsonError(.resourceLimit, "multipart part/count ceiling exceeded") }
            parts.append(HTTPRawPart(name: name, bytes: body.subdata(in: start..<end.lowerBound), headers: headers, filename: filename, contentType: types.first?.value))
            index = end.upperBound
        }
    }
    private static func boundaryLine(_ bytes: Data, at start: Int) -> (Bool, Int)? {
        var index = start
        let closing = bytes.count - index >= 2 && bytes[index] == 45 && bytes[index + 1] == 45
        if closing { index += 2 }
        while index < bytes.endIndex && (bytes[index] == 32 || bytes[index] == 9) { index += 1 }
        if closing && index == bytes.endIndex { return (true, index) }
        guard bytes.count - index >= 2 && bytes[index] == 13 && bytes[index + 1] == 10 else { return nil }
        return (closing, index + 2)
    }
    static func parseForm(_ body: Data, partLimit: Int) throws -> [HTTPRawPart] {
        let text = try HTTPWire.text(body)
        if text.isEmpty { return [] }
        return try text.split(separator: "&", omittingEmptySubsequences: false).map { pair in
            let fields = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
            guard fields.count == 2 else { throw JsonError(.representation, "form fields require name=value") }
            let name = try HTTPWire.decoded(String(fields[0]), form: true)
            let value = try HTTPWire.decoded(String(fields[1]), form: true)
            guard value.utf8.count <= partLimit else { throw JsonError(.resourceLimit, "form field ceiling exceeded") }
            return HTTPRawPart(name: name, bytes: Data(value.utf8), headers: [], filename: nil, contentType: nil)
        }
    }
}
