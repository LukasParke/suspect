import Foundation

/// Source-bound native conversion plus portable validation. Every encode checks
/// current mutable values, rather than trusting an earlier initializer/decoder.
public struct ModelCodec<Value: Sendable>: Sendable {
    public let source: SourceLocation
    let index: Int
    let decodeNative: @Sendable (JsonValue, inout ModelContext, String, Int) throws -> Value
    let encodeNative: @Sendable (Value, inout ModelContext, String, Int) throws -> JsonValue

    public func decode(_ data: Data, limits: JsonLimits = .init()) throws -> Value {
        try decodeParsed(JsonValue.parse(data, limits: limits), limits: limits)
    }
    public func decodeValue(_ value: JsonValue, limits: JsonLimits = .init()) throws -> Value {
        _ = try value.encoded(limits: limits) // also bound caller-constructed trees
        return try decodeParsed(value, limits: limits)
    }
    private func decodeParsed(_ value: JsonValue, limits: JsonLimits) throws -> Value {
        var context = ModelContext(limits: limits)
        try context.validation.check(index, value)
        return try decodeNative(value, &context, "", 0)
    }
    public func encode(_ value: Value, limits: JsonLimits = .init()) throws -> Data {
        try encodeValue(value, limits: limits).encoded(limits: limits)
    }
    public func encodeValue(_ value: Value, limits: JsonLimits = .init()) throws -> JsonValue {
        try limits.check()
        var context = ModelContext(limits: limits)
        let json = try encodeNative(value, &context, "", 0)
        _ = try json.encoded(limits: limits)
        try context.validation.check(index, json)
        return json
    }
}

/// Codable integration for source-backed models. Use SDKJSONDecoder/Encoder.
/// Foundation JSONDecoder cannot reveal original arbitrary number tokens;
/// these adapters reject it explicitly instead of advertising lossy fidelity.
public protocol SourceCodable: Codable, Sendable {
    static var codec: ModelCodec<Self> { get }
}
public extension SourceCodable {
    init(from decoder: any Decoder) throws {
        guard let decoder = decoder as? SourceDecoder else { throw sourceBoundary() }
        self = try Self.codec.decodeValue(decoder.value, limits: decoder.limits)
    }
    func encode(to encoder: any Encoder) throws {
        guard let encoder = encoder as? SourceEncoder else { throw sourceBoundary() }
        encoder.value = try Self.codec.encodeValue(self, limits: encoder.limits)
    }
}

/// A strict RFC 8259, source-preserving decoder for generated Codable models.
public struct SDKJSONDecoder: Sendable {
    public var limits: JsonLimits
    public init(limits: JsonLimits = .init()) { self.limits = limits }
    public func decode<Value: SourceCodable>(_ type: Value.Type, from data: Data) throws -> Value {
        try Value(from: SourceDecoder(value: JsonValue.parse(data, limits: limits), limits: limits))
    }
}
/// The matching source-preserving encoder. Optional keys are emitted by the
/// owning model codec; no generic Codable optional/null collapse is involved.
public struct SDKJSONEncoder: Sendable {
    public var limits: JsonLimits
    public init(limits: JsonLimits = .init()) { self.limits = limits }
    public func encode<Value: SourceCodable>(_ value: Value) throws -> Data {
        let encoder = SourceEncoder(limits: limits)
        try value.encode(to: encoder)
        guard !encoder.unsupported, let json = encoder.value else { throw sourceBoundary() }
        return try json.encoded(limits: limits)
    }
}

func sourceBoundary() -> JsonError {
    JsonError(.sourceBoundary, "generated models require the source-preserving SDKJSONDecoder/SDKJSONEncoder or their source-bound codec")
}
struct SourceDecoder: Decoder {
    let value: JsonValue
    let limits: JsonLimits
    var codingPath: [any CodingKey] { [] }
    var userInfo: [CodingUserInfoKey: Any] { [:] }
    func container<Key: CodingKey>(keyedBy type: Key.Type) throws -> KeyedDecodingContainer<Key> { throw sourceBoundary() }
    func unkeyedContainer() throws -> any UnkeyedDecodingContainer { throw sourceBoundary() }
    func singleValueContainer() throws -> any SingleValueDecodingContainer { throw sourceBoundary() }
}
final class SourceEncoder: Encoder {
    let limits: JsonLimits
    var value: JsonValue?
    var unsupported = false
    init(limits: JsonLimits) { self.limits = limits }
    var codingPath: [any CodingKey] { [] }
    var userInfo: [CodingUserInfoKey: Any] { [:] }
    func container<Key: CodingKey>(keyedBy type: Key.Type) -> KeyedEncodingContainer<Key> {
        unsupported = true; return KeyedEncodingContainer(RejectedKeyed<Key>(owner: self))
    }
    func unkeyedContainer() -> any UnkeyedEncodingContainer { unsupported = true; return RejectedValue(owner: self) }
    func singleValueContainer() -> any SingleValueEncodingContainer { unsupported = true; return RejectedValue(owner: self) }
}
// Encoder protocol container creation cannot throw. Unsupported paths record a
// failure and return throwing containers; they never trap or succeed silently.
struct RejectedValue: SingleValueEncodingContainer, UnkeyedEncodingContainer {
    let owner: SourceEncoder
    var codingPath: [any CodingKey] { [] }
    var count: Int { 0 }
    mutating func encodeNil() throws { throw sourceBoundary() }
    mutating func encode<T: Encodable>(_ value: T) throws { throw sourceBoundary() }
    mutating func encode(_ value: Bool) throws { throw sourceBoundary() }
    mutating func encode(_ value: String) throws { throw sourceBoundary() }
    mutating func encode(_ value: Double) throws { throw sourceBoundary() }
    mutating func encode(_ value: Float) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int8) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int16) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int32) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int64) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt8) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt16) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt32) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt64) throws { throw sourceBoundary() }
    mutating func nestedContainer<Key: CodingKey>(keyedBy type: Key.Type) -> KeyedEncodingContainer<Key> { owner.container(keyedBy: type) }
    mutating func nestedUnkeyedContainer() -> any UnkeyedEncodingContainer { owner.unkeyedContainer() }
    mutating func superEncoder() -> any Encoder { owner }
}
struct RejectedKeyed<Key: CodingKey>: KeyedEncodingContainerProtocol {
    let owner: SourceEncoder
    var codingPath: [any CodingKey] { [] }
    mutating func encodeNil(forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode<T: Encodable>(_ value: T, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Bool, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: String, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Double, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Float, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int8, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int16, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int32, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: Int64, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt8, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt16, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt32, forKey key: Key) throws { throw sourceBoundary() }
    mutating func encode(_ value: UInt64, forKey key: Key) throws { throw sourceBoundary() }
    mutating func nestedContainer<NestedKey: CodingKey>(keyedBy type: NestedKey.Type, forKey key: Key) -> KeyedEncodingContainer<NestedKey> { owner.container(keyedBy: type) }
    mutating func nestedUnkeyedContainer(forKey key: Key) -> any UnkeyedEncodingContainer { owner.unkeyedContainer() }
    mutating func superEncoder() -> any Encoder { owner }
    mutating func superEncoder(forKey key: Key) -> any Encoder { owner }
}

struct ModelContext {
    let limits: JsonLimits
    let validation = ValidationSession()
    var nodes = 0
    mutating func visit(_ path: String, _ depth: Int) throws {
        guard depth < limits.maxDepth && nodes < limits.maxNodes else { throw JsonError(.resourceLimit, "native conversion depth or node limit exceeded", path: path) }
        nodes += 1
    }
    func collection(_ count: Int, _ path: String) throws {
        guard count <= limits.maxNodes - nodes else { throw JsonError(.resourceLimit, "native collection exceeds remaining conversion nodes", path: path) }
    }
}

enum Conversion {
    typealias Decode<T> = (JsonValue, inout ModelContext, String, Int) throws -> T
    typealias Encode<T> = (T, inout ModelContext, String, Int) throws -> JsonValue
    static func required(_ value: JsonValue?, _ path: String) throws -> JsonValue {
        guard let value else { throw JsonError(.representation, "required property is absent", path: path) }
        return value
    }
    static func nonNull(_ value: JsonValue, _ path: String) throws -> JsonValue {
        guard !value.isNull else { throw JsonError(.representation, "null is not a non-null value", path: path) }
        return value
    }
    static func string(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> String {
        try context.visit(path, depth)
        guard case .string(let value) = value else { throw JsonError(.representation, "expected string", path: path) }
        return value
    }
    static func bool(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> Bool {
        try context.visit(path, depth)
        guard case .bool(let value) = value else { throw JsonError(.representation, "expected boolean", path: path) }
        return value
    }
    static func number(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonNumber {
        try context.visit(path, depth)
        guard case .number(let value) = value else { throw JsonError(.representation, "expected number", path: path) }
        return value
    }
    static func integer(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonInteger { try JsonInteger(number(value, &context, path, depth)) }
    static func null(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonNull {
        try context.visit(path, depth)
        guard value.isNull else { throw JsonError(.representation, "expected null", path: path) }
        return JsonNull()
    }
    static func any(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonValue { try context.visit(path, depth); return value }
    static func object(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int) throws -> JsonObject<JsonValue> {
        try context.visit(path, depth)
        guard case .object(let object) = value else { throw JsonError(.representation, "expected object", path: path) }
        return object
    }
    static func array<T>(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int, _ decode: Decode<T>) throws -> [T] {
        try context.visit(path, depth)
        guard case .array(let array) = value else { throw JsonError(.representation, "expected array", path: path) }
        try context.collection(array.count, path)
        return try array.enumerated().map { try decode($0.element, &context, childPath(path, String($0.offset)), depth + 1) }
    }
    static func nullable<T>(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int, _ decode: Decode<T>) throws -> Nullable<T> {
        try context.visit(path, depth)
        if value.isNull { return .null }
        return .value(try decode(value, &context, path, depth + 1))
    }
    static func optional<T>(_ value: JsonValue?, _ context: inout ModelContext, _ path: String, _ depth: Int, allowsNull: Bool, _ decode: Decode<T>) throws -> OptionalField<T> {
        guard let value else { return .missing }
        if !allowsNull { _ = try nonNull(value, path) }
        return .value(try decode(value, &context, path, depth))
    }
    static func presence<T>(_ value: JsonValue?, _ context: inout ModelContext, _ path: String, _ depth: Int, _ decode: Decode<T>) throws -> Presence<T> {
        guard let value else { return .missing }
        if value.isNull { return .null }
        return .value(try decode(value, &context, path, depth))
    }
    static func indirect<T>(_ value: JsonValue, _ context: inout ModelContext, _ path: String, _ depth: Int, _ decode: Decode<T>) throws -> Indirect<T> {
        try context.visit(path, depth)
        return .value(try decode(value, &context, path, depth + 1))
    }
    static func encode<T>(_ value: T, _ context: inout ModelContext, _ path: String, _ depth: Int, _ make: (T) -> JsonValue) throws -> JsonValue { try context.visit(path, depth); return make(value) }
    static func encodeArray<T>(_ value: [T], _ context: inout ModelContext, _ path: String, _ depth: Int, _ encode: Encode<T>) throws -> JsonValue {
        try context.visit(path, depth)
        try context.collection(value.count, path)
        return .array(try value.enumerated().map { try encode($0.element, &context, childPath(path, String($0.offset)), depth + 1) })
    }
    static func encodeNullable<T>(_ value: Nullable<T>, _ context: inout ModelContext, _ path: String, _ depth: Int, _ encode: Encode<T>) throws -> JsonValue {
        try context.visit(path, depth)
        switch value {
        case .null: return .null
        case .value(let value): return try nonNull(encode(value, &context, path, depth + 1), path)
        }
    }
    static func encodeIndirect<T>(_ value: Indirect<T>, _ context: inout ModelContext, _ path: String, _ depth: Int, _ encode: Encode<T>) throws -> JsonValue {
        try context.visit(path, depth)
        return try encode(value.wrappedValue, &context, path, depth + 1)
    }
}
