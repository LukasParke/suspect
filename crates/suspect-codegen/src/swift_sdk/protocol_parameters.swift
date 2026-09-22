import Foundation

enum HTTPScalar: Sendable { case string, boolean, integer, number }
enum HTTPStyle: Sendable { case simple, label, matrix, form, spaceDelimited, pipeDelimited, deepObject, cookie }
enum HTTPPercentEncoding: Sendable { case uriComponent, reservedExpansion, none, formUrlEncoded }
enum HTTPWireShape: Sendable {
    case scalar(HTTPScalar)
    case array(HTTPScalar)
    case object(properties: JsonObject<HTTPScalar>, additional: HTTPScalar?, allowAny: Bool)
}
enum HTTPSerialization: Sendable {
    case style(HTTPStyle, explode: Bool, shape: HTTPWireShape, encoding: HTTPPercentEncoding)
    case content(json: Bool, scalar: HTTPScalar, encoding: HTTPPercentEncoding)
}
struct HTTPParameter: Sendable {
    let name: String
    let location: String
    let required: Bool
    let source: SourceLocation
    let serialization: HTTPSerialization
}
struct HTTPWireFailure: Error, Sendable {
    let source: SourceLocation
    let error: JsonError
}

// Finite incremental writer. The counting pass proves the entire expansion fits
// before retaining URI/part output (including multiplicative name expansion).
struct HTTPWireBuffer {
    let limit: Int
    var counting = false
    private(set) var count = 0
    private var bytes: [UInt8] = []
    init(limit: Int, counting: Bool = false) { self.limit = limit; self.counting = counting }
    mutating func append(_ value: String) throws {
        let size = value.utf8.count
        guard size <= limit - count else { throw JsonError(.resourceLimit, "HTTP serialization byte limit exceeded") }
        count += size
        if !counting { bytes.append(contentsOf: value.utf8) }
    }
    mutating func byte(_ value: UInt8) throws {
        guard count < limit else { throw JsonError(.resourceLimit, "HTTP serialization byte limit exceeded") }
        count += 1
        if !counting { bytes.append(value) }
    }
    var string: String { String(decoding: bytes, as: UTF8.self) }
    var data: Data { Data(bytes) }
}

enum HTTPWire {
    static func token(_ value: String) -> Bool {
        !value.isEmpty && value.utf8.allSatisfy { (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || "!#$%&'*+-.^_`|~".utf8.contains($0) }
    }
    static func percentTriples(_ value: String) -> Bool {
        var bytes = value.utf8.makeIterator()
        while let byte = bytes.next() {
            if byte == 37 { guard let a = bytes.next(), let b = bytes.next(), hex(a) != nil, hex(b) != nil else { return false } }
        }
        return true
    }
    static func hex(_ b: UInt8) -> UInt8? {
        switch b { case 48...57: return b - 48; case 65...70: return b - 55; case 97...102: return b - 87; default: return nil }
    }
    static func splitQuoted(_ value: String, delimiter: Character) throws -> [String] {
        var result: [String] = []; var start = value.startIndex; var quoted = false; var escaped = false
        for index in value.indices {
            let ch = value[index]
            if escaped { escaped = false }
            else if quoted && ch == "\\" { escaped = true }
            else if ch == "\"" { quoted.toggle() }
            else if !quoted && ch == delimiter { result.append(String(value[start..<index])); start = value.index(after: index) }
        }
        guard !quoted && !escaped else { throw JsonError(.representation, "unterminated quoted field") }
        result.append(String(value[start...])); return result
    }
    static func unquote(_ raw: String) throws -> String {
        if !raw.hasPrefix("\"") {
            guard token(raw) else { throw JsonError(.representation, "invalid field token") }; return raw
        }
        guard raw.count >= 2, raw.hasSuffix("\"") else { throw JsonError(.representation, "unterminated quoted field") }
        var out = ""; var escaped = false
        for ch in raw.dropFirst().dropLast() {
            if escaped { out.append(ch); escaped = false }
            else if ch == "\\" { escaped = true }
            else if ch == "\"" { throw JsonError(.representation, "unescaped field quote") }
            else { out.append(ch) }
        }
        guard !escaped else { throw JsonError(.representation, "unterminated quoted pair") }; return out
    }
    static func scalar(_ value: JsonValue, type: HTTPScalar? = nil) throws -> String {
        switch (value, type) {
        case (.string(let s), .none), (.string(let s), .string): return s
        case (.bool(let b), .none), (.bool(let b), .boolean): return b ? "true" : "false"
        case (.number(let n), .none), (.number(let n), .number): return n.raw
        case (.number(let n), .integer): _ = try JsonInteger(n); return n.raw
        default: throw JsonError(.representation, "wire value must be a non-null scalar of the declared type")
        }
    }
    static func scalarValue(_ text: String, type: HTTPScalar) throws -> JsonValue {
        switch type {
        case .string: return .string(text)
        case .boolean:
            guard text == "true" || text == "false" else { throw JsonError(.representation, "invalid boolean text") }
            return .bool(text == "true")
        case .number: return .number(try JsonNumber(text))
        case .integer: return .number(try JsonInteger(text).number)
        }
    }
    static func text(_ data: Data) throws -> String {
        guard let value = String(data: data, encoding: .utf8) else { throw JsonError(.syntax, "HTTP text is not UTF-8") }; return value
    }
    static func writeEncoded(_ value: String, encoding: HTTPPercentEncoding, to out: inout HTTPWireBuffer) throws {
        if encoding == .none { try out.append(value); return }
        let bytes = value.utf8
        let digits = Array("0123456789ABCDEF".utf8)
        var index = bytes.startIndex
        while index != bytes.endIndex {
            let byte = bytes[index]
            let next = bytes.index(after: index)
            if encoding == .reservedExpansion && byte == 37 && next != bytes.endIndex {
                let end = bytes.index(after: next)
                if end != bytes.endIndex, hex(bytes[next]) != nil, hex(bytes[end]) != nil {
                    try out.byte(byte); try out.byte(bytes[next]); try out.byte(bytes[end]); index = bytes.index(after: end); continue
                }
            }
            let alpha = (65...90).contains(byte) || (97...122).contains(byte) || (48...57).contains(byte)
            let pass = encoding == .formUrlEncoded ? alpha || "*-._".utf8.contains(byte)
                : alpha || "-._~".utf8.contains(byte) || encoding == .reservedExpansion && ":/?#[]@!$&'()*+,;=".utf8.contains(byte)
            if pass { try out.byte(byte) }
            else if encoding == .formUrlEncoded && byte == 32 { try out.byte(43) }
            else { try out.byte(37); try out.byte(digits[Int(byte >> 4)]); try out.byte(digits[Int(byte & 15)]) }
            index = next
        }
    }
    static func encode(_ value: String, encoding: HTTPPercentEncoding, limit: Int) throws -> String {
        var count = HTTPWireBuffer(limit: limit, counting: true)
        try writeEncoded(value, encoding: encoding, to: &count)
        var result = HTTPWireBuffer(limit: count.count)
        try writeEncoded(value, encoding: encoding, to: &result); return result.string
    }
    static func decoded(_ value: String, form: Bool) throws -> String {
        guard percentTriples(value) else { throw JsonError(.representation, "invalid percent escape") }
        let text = form ? value.replacingOccurrences(of: "+", with: " ") : value
        guard let decoded = text.removingPercentEncoding else { throw JsonError(.syntax, "invalid encoded UTF-8") }; return decoded
    }
    static func checkValue(_ value: String, encoding: HTTPPercentEncoding, location: String, style: HTTPStyle?, composite: Bool) throws {
        if encoding == .none {
            guard !value.unicodeScalars.contains(where: { $0.properties.generalCategory == .control && !(location == "header" && $0.value == 9) }) else { throw JsonError(.representation, "control character in HTTP value") }
            if location == "cookie", value.utf8.contains(where: { $0 <= 32 || $0 > 126 || "\",;\\".utf8.contains($0) }) {
                throw JsonError(.representation, "cookie data must already escape disallowed octets")
            }
        }
        if style == .spaceDelimited && value.contains(" ") || style == .pipeDelimited && value.contains("|") || style == .deepObject && value.contains(where: { "[]".contains($0) }) {
            throw JsonError(.representation, "value contains a style delimiter requiring an API-defined escape")
        }
        if encoding == .reservedExpansion || encoding == .none && composite {
            let uriHazard = encoding == .reservedExpansion && (location == "query" && value.contains(where: { "#[]&=+".contains($0) })
                || location == "path" && value.contains(where: { "#[]/?".contains($0) }) || location == "cookie" && value.contains(where: { ";,".contains($0) }))
            let active: String
            switch style { case .label: active = ".,="; case .matrix: active = ";,="; case .simple, .form, .cookie: active = ",="; default: active = "" }
            if uriHazard || composite && value.contains(where: { active.contains($0) }) { throw JsonError(.representation, "value contains an active reserved delimiter") }
        }
    }
    static func empty(_ value: JsonValue) -> Bool {
        switch value { case .array(let a): return a.isEmpty; case .object(let o): return o.isEmpty; default: return false }
    }
    static func serialize(_ value: JsonValue, parameter: HTTPParameter, limit: Int) throws -> String {
        var counter = HTTPWireBuffer(limit: limit, counting: true)
        try serialize(value, parameter: parameter, to: &counter)
        var result = HTTPWireBuffer(limit: counter.count)
        try serialize(value, parameter: parameter, to: &result); return result.string
    }
    private static func serialize(_ value: JsonValue, parameter p: HTTPParameter, to out: inout HTTPWireBuffer) throws {
        switch p.serialization {
        case .content(let json, _, let encoding):
            let text = json ? try value.encodedString(limits: JsonLimits(maxBytes: out.limit)) : try scalar(value)
            try checkValue(text, encoding: encoding, location: p.location, style: nil, composite: false)
            if p.location == "query" || p.location == "cookie" { try writeEncoded(p.name, encoding: .uriComponent, to: &out); try out.append("=") }
            try writeEncoded(text, encoding: encoding, to: &out)
        case .style(let style, let explode, let shape, let encoding):
            let nameEncoding: HTTPPercentEncoding = p.location == "header" || style == .cookie ? .none : .uriComponent
            let single: String?; let items: [String]; let properties: [(String, String)]
            switch shape {
            case .scalar(let type): single = try scalar(value, type: type); items = []; properties = []
            case .array(let type):
                guard case .array(let array) = value, !array.isEmpty else { throw JsonError(.representation, "empty or non-array style value") }
                single = nil; items = try array.map { try scalar($0, type: type) }; properties = []
            case .object(let declared, let additional, let allowAny):
                guard case .object(let object) = value, !object.isEmpty else { throw JsonError(.representation, "empty or non-object style value") }
                single = nil; items = []
                properties = try object.members.map { key, value in
                    let type = declared[key] ?? additional
                    guard type != nil || allowAny else { throw JsonError(.representation, "undeclared style object property") }
                    return (key, try scalar(value, type: type))
                }
            }
            let composite = single == nil
            let data: (String, inout HTTPWireBuffer) throws -> Void = { value, output in
                try checkValue(value, encoding: encoding, location: p.location, style: style, composite: composite)
                try writeEncoded(value, encoding: encoding, to: &output)
            }
            let name: (inout HTTPWireBuffer) throws -> Void = { output in try writeEncoded(p.name, encoding: nameEncoding, to: &output) }
            let values: (String, Bool, inout HTTPWireBuffer) throws -> Void = { delimiter, paired, output in
                if let single { try data(single, &output) }
                else if case .array = shape {
                    for (i, item) in items.enumerated() { if i > 0 { try output.append(delimiter) }; try data(item, &output) }
                } else {
                    for (i, pair) in properties.enumerated() {
                        if i > 0 { try output.append(delimiter) }
                        try data(pair.0, &output); try output.append(paired ? "=" : delimiter); try data(pair.1, &output)
                    }
                }
            }
            switch style {
            case .simple: try values(",", explode, &out)
            case .label: try out.append("."); try values(explode ? "." : ",", explode, &out)
            case .matrix:
                if let single { try out.append(";"); try name(&out); if !single.isEmpty { try out.append("="); try data(single, &out) } }
                else if explode {
                    if case .array = shape {
                        for item in items { try out.append(";"); try name(&out); if !item.isEmpty { try out.append("="); try data(item, &out) } }
                    } else {
                        for pair in properties { try out.append(";"); try data(pair.0, &out); if !pair.1.isEmpty { try out.append("="); try data(pair.1, &out) } }
                    }
                } else { try out.append(";"); try name(&out); try out.append("="); try values(",", false, &out) }
            case .form, .cookie:
                let delimiter = style == .cookie ? "; " : "&"
                if composite && explode {
                    if case .array = shape {
                        for (i, item) in items.enumerated() { if i > 0 { try out.append(delimiter) }; try name(&out); try out.append("="); try data(item, &out) }
                    } else { try values(delimiter, true, &out) }
                } else { try name(&out); try out.append("="); try values(",", false, &out) }
            case .spaceDelimited, .pipeDelimited:
                try name(&out); try out.append("="); try values(style == .spaceDelimited ? "%20" : "%7C", false, &out)
            case .deepObject:
                for (i, pair) in properties.enumerated() {
                    if i > 0 { try out.append("&") }; try name(&out); try out.append("%5B"); try data(pair.0, &out); try out.append("%5D="); try data(pair.1, &out)
                }
            }
        }
    }
    static func header(_ headers: [HTTPHeader], parameter: HTTPParameter) throws -> JsonValue? {
        do { return try headerValue(headers, parameter: parameter) }
        catch let error as JsonError { throw HTTPWireFailure(source: parameter.source, error: error) }
    }
    private static func headerValue(_ headers: [HTTPHeader], parameter: HTTPParameter) throws -> JsonValue? {
        let fields = headers.filter { $0.name.lowercased() == parameter.name.lowercased() }
        if fields.isEmpty {
            guard !parameter.required else { throw JsonError(.representation, "required response/part header is missing") }; return nil
        }
        guard fields.count == 1 else { throw JsonError(.representation, "typed header requires one unambiguous field") }
        return try parseValue(fields[0].value, serialization: parameter.serialization)
    }
    static func parseValue(_ text: String, serialization: HTTPSerialization) throws -> JsonValue {
        switch serialization {
        case .content(let json, let scalar, _): return json ? try JsonValue.parse(text) : try scalarValue(text, type: scalar)
        case .style(_, let explode, let shape, _):
            switch shape {
            case .scalar(let type): return try scalarValue(text, type: type)
            case .array(let type): return .array(try text.split(separator: ",", omittingEmptySubsequences: false).map { try scalarValue(String($0), type: type) })
            case .object(let properties, let additional, let allowAny):
                let values = text.split(separator: ",", omittingEmptySubsequences: false)
                var pairs: [(String, String)] = []
                if explode {
                    for value in values {
                        let pair = value.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                        guard pair.count == 2 else { throw JsonError(.representation, "invalid exploded header object") }
                        pairs.append((String(pair[0]), String(pair[1])))
                    }
                } else {
                    guard values.count % 2 == 0 else { throw JsonError(.representation, "invalid header object") }
                    for i in stride(from: 0, to: values.count, by: 2) { pairs.append((String(values[i]), String(values[i + 1]))) }
                }
                return .object(try JsonObject(pairs.map { key, value in
                    guard let type = properties[key] ?? additional ?? (allowAny ? .string : nil) else { throw JsonError(.representation, "undeclared header object property") }
                    return (key, try scalarValue(value, type: type))
                }))
            }
        }
    }
    static func parsePartStyle(_ text: String, name: String, serialization: HTTPSerialization, multipart: Bool) throws -> JsonValue {
        guard case .style(let style, let explode, let shape, let encoding) = serialization else {
            return try parseValue(text, serialization: serialization)
        }
        let decode: (String) throws -> String = { value in encoding == .none ? value : try decoded(value, form: true) }
        let content: String
        switch style {
        case .form, .cookie:
            content = text
        default: content = text
        }
        switch shape {
        case .scalar(let scalar): return try scalarValue(decode(content), type: scalar)
        case .array(let scalar):
            let delimiter = style == .spaceDelimited ? (multipart ? " " : "%20") : style == .pipeDelimited ? (multipart ? "|" : "%7C") : ","
            let values: [String]
            if style == .form && explode && multipart {
                values = try text.components(separatedBy: "&").map { value in
                    guard value.hasPrefix(name + "=") else { throw JsonError(.representation, "unexpected exploded part name") }
                    return String(value.dropFirst(name.count + 1))
                }
            } else { values = content.components(separatedBy: delimiter) }
            return .array(try values.map { try scalarValue(decode($0), type: scalar) })
        case .object(let properties, let additional, let allowAny):
            var pairs: [(String, String)] = []
            if explode && (style == .form || style == .deepObject) {
                for field in text.components(separatedBy: "&") {
                    let pieces = field.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                    guard pieces.count == 2 else { throw JsonError(.representation, "invalid styled object part") }
                    var key = String(pieces[0])
                    if style == .deepObject {
                        guard key.hasPrefix(name + "%5B"), key.hasSuffix("%5D") else { throw JsonError(.representation, "invalid deep object part") }
                        key = String(key.dropFirst(name.count + 3).dropLast(3))
                    }
                    pairs.append((try decode(key), try decode(String(pieces[1]))))
                }
            } else {
                let delimiter = style == .spaceDelimited ? (multipart ? " " : "%20") : style == .pipeDelimited ? (multipart ? "|" : "%7C") : ","
                let fields = content.components(separatedBy: delimiter)
                if explode {
                    for field in fields {
                        let p = field.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
                        guard p.count == 2 else { throw JsonError(.representation, "invalid styled object part") }
                        pairs.append((try decode(String(p[0])), try decode(String(p[1]))))
                    }
                } else {
                    guard fields.count % 2 == 0 else { throw JsonError(.representation, "invalid styled object part") }
                    for i in stride(from: 0, to: fields.count, by: 2) { pairs.append((try decode(fields[i]), try decode(fields[i + 1]))) }
                }
            }
            return .object(try JsonObject(pairs.map { key, value in
                guard let scalar = properties[key] ?? additional ?? (allowAny ? .string : nil) else { throw JsonError(.representation, "undeclared styled object property") }
                return (key, try scalarValue(value, type: scalar))
            }))
        }
    }
    /// RFC6570 multipart/form-data puts the query name in Content-Disposition
    /// and only its value in the body. There is no URI encoding or name=value
    /// wrapper. Multi-field expansions require an explicit grouping plan.
    static func multipartContent(_ value: JsonValue, serialization: HTTPSerialization, limit: Int) throws -> String {
        guard case .style(let style, let explode, let shape, _) = serialization else { throw JsonError(.representation, "multipart style descriptor required") }
        let values: [String]
        switch shape {
        case .scalar(let type): values = [try scalar(value, type: type)]
        case .array(let type):
            guard !explode, case .array(let array) = value, !array.isEmpty else { throw JsonError(.representation, "multipart style cannot change positional/grouped part cardinality") }
            values = try array.map { try scalar($0, type: type) }
        case .object(let properties, let additional, let allowAny):
            guard !explode, style != .deepObject, case .object(let object) = value, !object.isEmpty else { throw JsonError(.representation, "multipart style requires an explicit multi-part grouping plan") }
            values = try object.members.flatMap { key, value -> [String] in
                let type = properties[key] ?? additional
                guard type != nil || allowAny else { throw JsonError(.representation, "undeclared style object property") }
                return [key, try scalar(value, type: type)]
            }
        }
        let separator = style == .spaceDelimited ? " " : style == .pipeDelimited ? "|" : ","
        var output = HTTPWireBuffer(limit: limit)
        for (index, text) in values.enumerated() {
            try checkValue(text, encoding: .none, location: "part", style: style, composite: values.count > 1)
            if index > 0 { try output.append(separator) }; try output.append(text)
        }
        return output.string
    }
}
