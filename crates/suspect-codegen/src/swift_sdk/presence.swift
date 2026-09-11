/// Optional, non-null field. A missing key is distinct from a present value.
/// The indirect case gives recursive structs finite, value-semantic storage.
public indirect enum OptionalField<Value: Sendable>: Sendable {
    case missing
    case value(Value)
    public var valueIfPresent: Value? { if case .value(let value) = self { return value }; return nil }
}
extension OptionalField: Equatable where Value: Equatable {}

/// Required nullable value: omission is not representable.
public indirect enum Nullable<Value: Sendable>: Sendable {
    case null
    case value(Value)
    public var valueIfPresent: Value? { if case .value(let value) = self { return value }; return nil }
}
extension Nullable: Equatable where Value: Equatable {}

/// Optional nullable field. Missing, explicit null, and a value round trip as
/// three distinct states; codecs reject a null nested inside `.value`.
public indirect enum Presence<Value: Sendable>: Sendable {
    case missing
    case null
    case value(Value)
    public var valueIfPresent: Value? { if case .value(let value) = self { return value }; return nil }
}
extension Presence: Equatable where Value: Equatable {}

/// Value-semantic indirection used only for required object layout cycles.
public indirect enum Indirect<Value: Sendable>: Sendable {
    case value(Value)
    public var wrappedValue: Value {
        get { switch self { case .value(let value): return value } }
        set { self = .value(newValue) }
    }
}
extension Indirect: Equatable where Value: Equatable {}
