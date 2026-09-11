import Foundation

/// Finite exact-JSON limits. Each parse/write call owns its counters.
public struct JsonLimits: Sendable {
    public var maxBytes: Int
    public var maxDepth: Int
    public var maxNodes: Int
    public var maxNumberBytes: Int

    public init(maxBytes: Int = 8 * 1024 * 1024, maxDepth: Int = 128,
                maxNodes: Int = 100_000, maxNumberBytes: Int = 4096) {
        self.maxBytes = maxBytes
        self.maxDepth = maxDepth
        self.maxNodes = maxNodes
        self.maxNumberBytes = maxNumberBytes
    }

    func check() throws {
        guard (1...Int(Int32.max)).contains(maxBytes), (1...512).contains(maxDepth),
              (1...1_000_000).contains(maxNodes), (1...65_536).contains(maxNumberBytes) else {
            throw JsonError(.resourceLimit, "invalid JSON resource policy")
        }
    }
}

/// Syntax/representation failure, separate from a schema mismatch.
public struct JsonError: Error, Sendable, CustomStringConvertible {
    public enum Kind: String, Sendable { case syntax, resourceLimit, duplicateKey, invalidNumber, sourceBoundary, representation }
    public let kind: Kind
    public let message: String
    public let offset: Int?
    public let path: String
    public var description: String { "\(kind.rawValue): \(message) at \(path)\(offset.map { " byte \($0)" } ?? "")" }
    init(_ kind: Kind, _ message: String, offset: Int? = nil, path: String = "") {
        self.kind = kind; self.message = message; self.offset = offset; self.path = path
    }
}

/// An RFC 8259 number token. Construction validates grammar and never uses a
/// floating-point or Foundation NSNumber intermediate. Equality is token equality.
public struct JsonNumber: Sendable, Equatable, Hashable, ExpressibleByIntegerLiteral {
    public let raw: String
    public init(_ raw: String) throws {
        guard raw.utf8.count <= 65_536 else { throw JsonError(.resourceLimit, "number token exceeds 65536 bytes") }
        let bytes = Array(raw.utf8)
        var parser = JSONParser(bytes: bytes, limits: JsonLimits(maxNumberBytes: 65_536))
        _ = try parser.number()
        guard parser.at == bytes.count else { throw JsonError(.invalidNumber, "trailing bytes in number token", offset: parser.at) }
        self.raw = raw
    }
    public init(raw: String) throws { try self.init(raw) }
    public init(integerLiteral value: Int64) { self.raw = String(value) }
    public init(_ value: Int64) { self.raw = String(value) }
    // Only generated, compiler-checked literals and the strict parser use this.
    init(trusted raw: String) { self.raw = raw }
}

/// An exact mathematical integer, retaining spellings such as 1.00e+3.
public struct JsonInteger: Sendable, Equatable, Hashable, ExpressibleByIntegerLiteral {
    public let number: JsonNumber
    public var raw: String { number.raw }
    public init(_ raw: String) throws { try self.init(JsonNumber(raw)) }
    public init(_ number: JsonNumber) throws {
        guard ExactDecimal(number.raw).isIntegral else { throw JsonError(.invalidNumber, "integer value has a fractional component") }
        self.number = number
    }
    public init(integerLiteral value: Int64) { number = JsonNumber(value) }
    public init(_ value: Int64) { number = JsonNumber(value) }
    /// Checked narrowing. Out-of-range values throw; no zero substitution occurs.
    public func int64Value() throws -> Int64 {
        let exact = ExactDecimal(raw)
        if exact.sign == 0 { return 0 }
        guard exact.exponent.compare(BigSigned(19)) <= 0,
              let shift = exact.exponent.smallInt(), shift >= 0,
              exact.digits.count + shift <= 19 else { throw JsonError(.representation, "integer is outside Int64") }
        let text = (exact.sign < 0 ? "-" : "") + String(decoding: exact.digits.map { $0 + 48 }, as: UTF8.self) + String(repeating: "0", count: shift)
        guard let value = Int64(text) else { throw JsonError(.representation, "integer is outside Int64") }
        return value
    }
}

/// The only value of a source-declared null-only schema.
public struct JsonNull: Sendable, Equatable { public init() {} }

// Swift.String equality normalizes Unicode. JSON property identities must not:
// U+00E9 and U+0065 U+0301 are different names under JSON Schema.
struct JsonKey: Sendable, Hashable {
    let text: String
    init(_ text: String) { self.text = text }
    static func == (a: Self, b: Self) -> Bool { a.text.utf8.elementsEqual(b.text.utf8) }
    func hash(into hasher: inout Hasher) {
        hasher.combine(text.utf8.count)
        for byte in text.utf8 { hasher.combine(byte) }
    }
}

/// A value-semantic JSON object with exact Unicode key identity. Keys are sorted
/// by UTF-8 for deterministic encoding; mutation preserves every distinct name.
public struct JsonObject<Value: Sendable>: Sendable {
    private var storage: [JsonKey: Value] = [:]
    public init() {}
    public init(_ members: [(String, Value)]) throws {
        for (key, value) in members {
            guard storage.updateValue(value, forKey: JsonKey(key)) == nil else {
                throw JsonError(.duplicateKey, "duplicate decoded object key")
            }
        }
    }
    init(trusted members: [(String, Value)]) {
        for (key, value) in members { storage[JsonKey(key)] = value }
    }
    public subscript(key: String) -> Value? {
        get { storage[JsonKey(key)] }
        set { storage[JsonKey(key)] = newValue }
    }
    public var count: Int { storage.count }
    public var isEmpty: Bool { storage.isEmpty }
    public var members: [(key: String, value: Value)] {
        storage.map { (key: $0.key.text, value: $0.value) }.sorted { $0.key.utf8.lexicographicallyPrecedes($1.key.utf8) }
    }
    public var keys: [String] { members.map(\.key) }
}
extension JsonObject: Equatable where Value: Equatable {
    public static func == (a: Self, b: Self) -> Bool { a.storage == b.storage }
}

/// Source-preserving JSON values. Numeric tokens and Unicode scalar sequences
/// survive round trips; whitespace, key order and string escape spellings may change.
public indirect enum JsonValue: Sendable, Equatable {
    case null
    case bool(Bool)
    case number(JsonNumber)
    case string(String)
    case array([JsonValue])
    case object(JsonObject<JsonValue>)

    public static func == (a: Self, b: Self) -> Bool {
        switch (a, b) {
        case (.null, .null): return true
        case let (.bool(a), .bool(b)): return a == b
        case let (.number(a), .number(b)): return a == b
        case let (.string(a), .string(b)): return a.utf8.elementsEqual(b.utf8)
        case let (.array(a), .array(b)): return a == b
        case let (.object(a), .object(b)): return a == b
        default: return false
        }
    }
    public static func parse(_ data: Data, limits: JsonLimits = .init()) throws -> Self {
        try limits.check()
        guard data.count <= limits.maxBytes else { throw JsonError(.resourceLimit, "JSON input byte limit exceeded") }
        var parser = JSONParser(bytes: Array(data), limits: limits)
        let result = try parser.value(depth: 0)
        parser.space()
        guard parser.at == data.count else { throw JsonError(.syntax, "trailing bytes after JSON value", offset: parser.at) }
        return result
    }
    public static func parse(_ text: String, limits: JsonLimits = .init()) throws -> Self {
        try limits.check()
        guard text.utf8.count <= limits.maxBytes else { throw JsonError(.resourceLimit, "JSON input byte limit exceeded") }
        return try parse(Data(text.utf8), limits: limits)
    }
    public func encoded(limits: JsonLimits = .init()) throws -> Data {
        try limits.check()
        var writer = JSONWriter(limits: limits)
        try writer.value(self, depth: 0)
        return Data(writer.bytes)
    }
    public func encodedString(limits: JsonLimits = .init()) throws -> String {
        String(decoding: try encoded(limits: limits), as: UTF8.self)
    }
    var isNull: Bool { if case .null = self { return true }; return false }
    var kind: String {
        switch self {
        case .null: return "null"
        case .bool: return "boolean"
        case .number: return "number"
        case .string: return "string"
        case .array: return "array"
        case .object: return "object"
        }
    }
}

/// Compatibility spelling for the exact JSON value domain.
public typealias ExactValue = JsonValue

struct JSONParser {
    let bytes: [UInt8]
    let limits: JsonLimits
    var at = 0
    var nodes = 0

    mutating func space() {
        while at < bytes.count && [UInt8(32), 9, 10, 13].contains(bytes[at]) { at += 1 }
    }
    func error(_ message: String) -> JsonError { JsonError(.syntax, message, offset: at) }
    mutating func take(_ byte: UInt8) -> Bool {
        guard at < bytes.count && bytes[at] == byte else { return false }
        at += 1; return true
    }
    mutating func value(depth: Int) throws -> JsonValue {
        guard depth < limits.maxDepth && nodes < limits.maxNodes else { throw JsonError(.resourceLimit, "JSON depth or node limit exceeded", offset: at) }
        nodes += 1; space()
        guard at < bytes.count else { throw error("expected JSON value") }
        switch bytes[at] {
        case 110: try literal("null"); return .null
        case 116: try literal("true"); return .bool(true)
        case 102: try literal("false"); return .bool(false)
        case 34: return .string(try string())
        case 91:
            at += 1; space()
            var items: [JsonValue] = []
            if take(93) { return .array(items) }
            while true {
                items.append(try value(depth: depth + 1)); space()
                if take(93) { return .array(items) }
                guard take(44) else { throw error("expected ',' or ']'") }
            }
        case 123:
            at += 1; space()
            var object = JsonObject<JsonValue>()
            if take(125) { return .object(object) }
            while true {
                space()
                guard at < bytes.count && bytes[at] == 34 else { throw error("expected object key") }
                let key = try string(); space()
                guard take(58) else { throw error("expected ':'") }
                guard object[key] == nil else { throw JsonError(.duplicateKey, "duplicate decoded object key", offset: at) }
                object[key] = try value(depth: depth + 1); space()
                if take(125) { return .object(object) }
                guard take(44) else { throw error("expected ',' or '}'") }
            }
        default: return .number(try number())
        }
    }
    mutating func literal(_ text: String) throws {
        for byte in text.utf8 { guard take(byte) else { throw error("invalid literal") } }
    }
    mutating func number() throws -> JsonNumber {
        let start = at
        _ = take(45)
        guard at < bytes.count else { throw error("incomplete number") }
        if take(48) {
            if at < bytes.count && (48...57).contains(bytes[at]) { throw error("leading zero in number") }
        } else {
            guard (49...57).contains(bytes[at]) else { throw error("invalid number") }
            while at < bytes.count && (48...57).contains(bytes[at]) { at += 1 }
        }
        if take(46) {
            let digits = at
            while at < bytes.count && (48...57).contains(bytes[at]) { at += 1 }
            guard at > digits else { throw error("fraction requires digits") }
        }
        if take(101) || take(69) {
            if !take(43) { _ = take(45) }
            let digits = at
            while at < bytes.count && (48...57).contains(bytes[at]) { at += 1 }
            guard at > digits else { throw error("exponent requires digits") }
        }
        guard at - start <= limits.maxNumberBytes else { throw JsonError(.resourceLimit, "number byte limit exceeded", offset: start) }
        return JsonNumber(trusted: String(decoding: bytes[start..<at], as: UTF8.self))
    }
    mutating func hex() throws -> UInt32 {
        var value: UInt32 = 0
        for _ in 0..<4 {
            guard at < bytes.count else { throw error("incomplete Unicode escape") }
            let byte = bytes[at]; at += 1
            let digit: UInt32
            switch byte {
            case 48...57: digit = UInt32(byte - 48)
            case 65...70: digit = UInt32(byte - 55)
            case 97...102: digit = UInt32(byte - 87)
            default: throw error("invalid Unicode escape")
            }
            value = value * 16 + digit
        }
        return value
    }
    mutating func string() throws -> String {
        guard take(34) else { throw error("expected string") }
        var out: [UInt8] = []
        while at < bytes.count {
            let byte = bytes[at]; at += 1
            if byte == 34 {
                guard let text = String(bytes: out, encoding: .utf8) else { throw error("invalid UTF-8") }
                return text
            }
            if byte == 92 {
                guard at < bytes.count else { throw error("incomplete escape") }
                let escaped = bytes[at]; at += 1
                switch escaped {
                case 34, 47, 92: out.append(escaped)
                case 98: out.append(8)
                case 102: out.append(12)
                case 110: out.append(10)
                case 114: out.append(13)
                case 116: out.append(9)
                case 117:
                    var scalar = try hex()
                    if (0xd800...0xdbff).contains(scalar) {
                        guard take(92), take(117) else { throw error("high surrogate requires a low surrogate") }
                        let low = try hex()
                        guard (0xdc00...0xdfff).contains(low) else { throw error("invalid low surrogate") }
                        scalar = 0x10000 + (scalar - 0xd800) * 0x400 + low - 0xdc00
                    }
                    guard let unicode = Unicode.Scalar(scalar) else { throw error("unpaired surrogate") }
                    out.append(contentsOf: String(unicode).utf8)
                default: throw error("invalid escape")
                }
            } else {
                guard byte >= 32 else { throw error("unescaped control character") }
                out.append(byte)
            }
        }
        throw error("unterminated string")
    }
}

struct JSONWriter {
    let limits: JsonLimits
    var bytes: [UInt8] = []
    var nodes = 0
    mutating func append<S: Sequence>(_ values: S) throws where S.Element == UInt8 {
        for byte in values {
            guard bytes.count < limits.maxBytes else { throw JsonError(.resourceLimit, "JSON output byte limit exceeded") }
            bytes.append(byte)
        }
    }
    mutating func byte(_ value: UInt8) throws { try append(CollectionOfOne(value)) }
    mutating func quoted(_ text: String) throws {
        try byte(34)
        for scalar in text.unicodeScalars {
            switch scalar.value {
            case 34: try append("\\\"".utf8)
            case 92: try append("\\\\".utf8)
            case 8: try append("\\b".utf8)
            case 9: try append("\\t".utf8)
            case 10: try append("\\n".utf8)
            case 12: try append("\\f".utf8)
            case 13: try append("\\r".utf8)
            case 0..<32:
                let hex = String(scalar.value, radix: 16)
                try append(("\\u" + String(repeating: "0", count: 4 - hex.count) + hex).utf8)
            default: try append(String(scalar).utf8)
            }
        }
        try byte(34)
    }
    mutating func value(_ value: JsonValue, depth: Int) throws {
        guard depth < limits.maxDepth && nodes < limits.maxNodes else { throw JsonError(.resourceLimit, "JSON depth or node limit exceeded") }
        nodes += 1
        switch value {
        case .null: try append("null".utf8)
        case .bool(let b): try append((b ? "true" : "false").utf8)
        case .number(let n):
            guard n.raw.utf8.count <= limits.maxNumberBytes else { throw JsonError(.resourceLimit, "number byte limit exceeded") }
            try append(n.raw.utf8)
        case .string(let s): try quoted(s)
        case .array(let items):
            try byte(91)
            for (i, item) in items.enumerated() {
                if i > 0 { try byte(44) }
                try self.value(item, depth: depth + 1)
            }
            try byte(93)
        case .object(let object):
            guard object.count <= limits.maxNodes - nodes else { throw JsonError(.resourceLimit, "JSON object exceeds remaining node limit") }
            try byte(123)
            for (i, member) in object.members.enumerated() {
                if i > 0 { try byte(44) }
                try quoted(member.key); try byte(58)
                try self.value(member.value, depth: depth + 1)
            }
            try byte(125)
        }
    }
}
