import Foundation

/// Stable original document URI and RFC 6901 pointer.
public struct SourceLocation: Sendable, Equatable, CustomStringConvertible {
    public let document: String
    public let pointer: String
    public var description: String { document + "#" + pointer }
    public init(document: String, pointer: String) { self.document = document; self.pointer = pointer }
}

/// Completed schema invalidity and incomplete evaluation are distinct outcomes.
/// Resource failures are never suppressed by a successful union alternative.
public struct ValidationError: Error, Sendable, CustomStringConvertible {
    public enum Kind: String, Sendable { case invalid, evaluationFailure }
    public let kind: Kind
    public let source: SourceLocation
    public let instancePath: String
    public let message: String
    public var description: String { "\(kind.rawValue) at \(source) \(instancePath): \(message)" }
}

struct ValidationNode: Sendable { let source: SourceLocation; let checks: [ValidationCheck] }
struct ValidationCheck: Sendable { let source: SourceLocation; let instruction: ValidationInstruction }
enum ValidationInstruction: Sendable {
    case always(Bool), type([String]), reference(Int)
    case dynamicReference(target: Int, initialResource: Int, anchor: String?)
    case properties([(String, Int)]), additionalProperties(declared: [String], target: Int), required([String])
    case items(target: Int, start: Int), prefixItems([Int])
    case allOf([Int]), anyOf([Int]), oneOf([Int]), not(Int)
    case bound(value: String, maximum: Bool, exclusive: Bool), multipleOf(String)
    case count(value: String, maximum: Bool, target: String)
    case enumeration([JsonValue]), constant(JsonValue), uniqueItems, pattern(PatternProgram)
    case conditional(condition: Int, then: Int?, otherwise: Int?)
    case dependentRequired([(String, [String])]), dependentSchemas([(String, Int)])
    case contains(target: Int, minimum: String?, maximum: String?)
    case patternProperties([(String, PatternProgram, Int)])
    case additionalPropertiesWithPatterns(declared: [String], target: Int)
    case propertyNames(Int), unevaluatedProperties(Int), unevaluatedItems(Int)
}
// Copied from the checked, indexed Contract resource graph. Logical URI strings
// are metadata only: native validation performs no URI resolution or acquisition.
struct ValidationResourceContext: Sendable {
    let resources: [ValidationResource]
    let nodeScopes: [ValidationNodeScope]
}
struct ValidationResource: Sendable {
    let source: SourceLocation
    let kind: String
    let canonicalURI: String
    let baseURI: String
    let aliases: [String]
    let declarationSource: SourceLocation?
    let dynamicAnchors: [(String, SourceLocation, Int)]
}
struct ValidationNodeScope: Sendable {
    let resource: Int
    let schemaRoot: SourceLocation
    let canonicalAddress: String
}
struct PatternProgram: Sendable { let start: Int; let states: [PatternState] }
enum PatternState: Sendable {
    case match, char(ranges: [(UInt32, UInt32)], target: Int), split(Int, Int), jump(Int), start(Int), end(Int)
}

func childPath(_ path: String, _ key: String) -> String {
    path + "/" + key.replacingOccurrences(of: "~", with: "~0").replacingOccurrences(of: "/", with: "~1")
}

// One per top-level codec call. Generated program tables are immutable and
// Sendable; no validation state is shared across requests or Swift tasks.
final class ValidationSession {
    private var steps = ValidationProgram.maxEvaluationSteps
    private var equalities = ValidationProgram.maxEqualitySteps
    private var numeric = ValidationProgram.maxEvaluationSteps
    private var finding: ValidationError?
    private struct Active: Hashable { let node: Int; let path: JsonKey; let context: Int }
    private var active: Set<Active> = []
    private struct ContextTransition: Hashable { let previous: Int; let resource: Int }
    private var contexts: [ContextTransition: Int] = [:]
    private var context = 0
    private var resourceStack: [Int] = []
    private var enteredResources: Set<Int> = []
    private struct Annotations {
        var properties: Set<JsonKey> = []
        var items: Set<Int> = []
    }
    private struct Evaluated {
        let valid: Bool
        let annotations: Annotations
    }

    func check(_ index: Int, _ value: JsonValue, path: String = "") throws {
        finding = nil
        if try !evaluate(index, value, path, 0).valid {
            throw finding ?? ValidationError(kind: .invalid, source: ValidationProgram.nodes[index].source, instancePath: path, message: "source schema rejected the value")
        }
    }
    func matches(_ index: Int, _ value: JsonValue, path: String = "") throws -> Bool {
        let saved = finding
        defer { finding = saved }
        return try evaluate(index, value, path, 0).valid
    }
    private func failure(_ source: SourceLocation, _ path: String, _ message: String) -> ValidationError {
        ValidationError(kind: .evaluationFailure, source: source, instancePath: path, message: message)
    }
    private func spend(_ source: SourceLocation, _ path: String, _ amount: Int = 1) throws {
        guard amount <= steps else { throw failure(source, path, "evaluation work budget exhausted") }
        steps -= amount
    }
    private func mismatch(_ source: SourceLocation, _ path: String, _ message: String) -> Bool {
        if finding == nil { finding = ValidationError(kind: .invalid, source: source, instancePath: path, message: message) }
        return false
    }
    private func number(_ token: String, _ source: SourceLocation, _ path: String) throws -> ExactDecimal {
        guard token.utf8.count <= ValidationProgram.maxNumberBytes else { throw failure(source, path, "numeric operand byte budget exhausted") }
        return ExactDecimal(token)
    }
    private func trial(_ index: Int, _ value: JsonValue, _ path: String, _ depth: Int) throws -> Evaluated {
        let saved = finding
        defer { finding = saved }
        return try evaluate(index, value, path, depth)
    }
    private func mark(_ name: String, in annotations: inout Annotations) {
        if ValidationProgram.collectAnnotations { annotations.properties.insert(JsonKey(name)) }
    }
    private func mark(_ index: Int, in annotations: inout Annotations) {
        if ValidationProgram.collectAnnotations { annotations.items.insert(index) }
    }
    private func merge(_ other: Annotations, into annotations: inout Annotations, source: SourceLocation, path: String) throws {
        if !ValidationProgram.collectAnnotations { return }
        for property in other.properties { try spend(source, path); annotations.properties.insert(property) }
        for item in other.items { try spend(source, path); annotations.items.insert(item) }
    }
    private func keyword(_ source: SourceLocation, _ name: String) -> SourceLocation {
        SourceLocation(document: source.document, pointer: childPath(source.pointer, name))
    }
    private func adjacentPatterns(_ node: ValidationNode) -> [(String, PatternProgram, Int)] {
        for check in node.checks { if case .patternProperties(let patterns) = check.instruction { return patterns } }
        return [] // The checked program permits this only when no patterns exist.
    }
    private func enterResource(_ index: Int, _ source: SourceLocation, _ path: String) throws -> Int? {
        guard let graph = ValidationProgram.resourceContext else { return nil }
        let resource = graph.nodeScopes[index].resource
        if enteredResources.contains(resource) { return nil }
        try spend(source, path)
        let previous = context
        let transition = ContextTransition(previous: previous, resource: resource)
        if let known = contexts[transition] { context = known }
        else {
            // Each new identity consumes an evaluation step. The checked Int32
            // work ceiling bounds this exact interning table and its Int IDs.
            context = contexts.count + 1
            contexts[transition] = context
        }
        enteredResources.insert(resource)
        resourceStack.append(resource)
        return previous
    }
    private func leaveResource(_ previous: Int?) {
        if let previous {
            enteredResources.remove(resourceStack.removeLast())
            context = previous
        }
    }
    @inline(never) private func evaluate(_ index: Int, _ value: JsonValue, _ path: String, _ depth: Int) throws -> Evaluated {
        let node = ValidationProgram.nodes[index]
        if ValidationProgram.resourceContext != nil { try spend(node.source, path) }
        guard depth < ValidationProgram.maxDepth else { throw failure(node.source, path, "evaluation depth budget exhausted") }
        let entered = try enterResource(index, node.source, path)
        defer { leaveResource(entered) }
        let identity = Active(node: index, path: JsonKey(path), context: context)
        guard active.insert(identity).inserted else { throw failure(node.source, path, "nonproductive recursive evaluation") }
        defer { active.remove(identity) }
        if ValidationProgram.resourceContext == nil { try spend(node.source, path) }
        var valid = true
        var annotations = Annotations()
        for check in node.checks {
            let source = check.source
            try spend(source, path)
            var produced = Annotations()
            let rule = handler(check, node, value, path, depth)
            let result = try rule(&annotations, &produced)
            if !result { _ = mismatch(source, path, "source assertion rejected the value") }
            valid = result && valid
            if result { try merge(produced, into: &annotations, source: source, path: path) }
        }
        return Evaluated(valid: valid, annotations: valid ? annotations : Annotations())
    }
    // The selection frame returns before a rule can recurse. Swift debug builds
    // otherwise reserve every enum payload temporary on each recursive stack.
    private typealias Rule = (inout Annotations, inout Annotations) throws -> Bool
    @inline(never) private func handler(_ check: ValidationCheck, _ node: ValidationNode, _ value: JsonValue, _ path: String, _ depth: Int) -> Rule {
        let source = check.source
        switch check.instruction {
        case .always(let truth): return { _, _ in truth }
        case .type(let types): return { [self] _, _ in try type(types, value, source, path) }
        case .reference(let target): return { [self] _, produced in try reference(target, value, path, depth, &produced) }
        case .dynamicReference(let target, _, let anchor):
            return { [self] _, produced in
                let selected = try dynamicTarget(target, anchor, source, path)
                return try reference(selected, value, path, depth, &produced)
            }
        case .properties(let properties): return { [self] _, produced in try propertiesRule(properties, value, source, path, depth, &produced) }
        case .additionalProperties(let names, let target): return { [self] _, produced in try additional(names, target, value, source, path, depth, &produced) }
        case .required(let names): return { [self] _, _ in try required(names, value, source, path) }
        case .items(let target, let start): return { [self] _, produced in try items(target, start, value, source, path, depth, &produced) }
        case .prefixItems(let targets): return { [self] _, produced in try prefix(targets, value, source, path, depth, &produced) }
        case .allOf(let targets), .anyOf(let targets), .oneOf(let targets): return { [self] _, produced in try composition(check.instruction, targets, value, source, path, depth, &produced) }
        case .not(let target): return { [self] _, _ in try !trial(target, value, path, depth + 1).valid }
        case .conditional(let condition, let yes, let no): return { [self] local, produced in try conditional(condition, yes, no, value, source, path, depth, &local, &produced) }
        case .dependentRequired(let dependencies): return { [self] _, _ in try dependentRequired(dependencies, value, source, path) }
        case .dependentSchemas(let dependencies): return { [self] _, produced in try dependentSchemas(dependencies, value, source, path, depth, &produced) }
        case .contains(let target, let min, let max): return { [self] local, _ in try contains(target, min, max, value, node.source, source, path, depth, &local) }
        case .patternProperties(let patterns): return { [self] _, produced in try patternProperties(patterns, value, source, path, depth, &produced) }
        case .additionalPropertiesWithPatterns(let names, let target): return { [self] _, produced in try additionalPatterns(names, target, adjacentPatterns(node), value, source, path, depth, &produced) }
        case .propertyNames(let target): return { [self] _, _ in try propertyNames(target, value, source, path, depth) }
        case .unevaluatedProperties(let target): return { [self] local, produced in try unevaluatedProperties(target, value, source, path, depth, local, &produced) }
        case .unevaluatedItems(let target): return { [self] local, produced in try unevaluatedItems(target, value, source, path, depth, local, &produced) }
        case .bound(let token, let maximum, let exclusive): return { [self] _, _ in try bound(token, maximum, exclusive, value, source, path) }
        case .multipleOf(let token): return { [self] _, _ in try multiple(token, value, source, path) }
        case .count(let token, let maximum, let target): return { [self] _, _ in cardinality(token, maximum, target, value) }
        case .enumeration(let candidates): return { [self] _, _ in try enumeration(candidates, value, source, path) }
        case .constant(let candidate): return { [self] _, _ in try equal(value, candidate, source, path, 0) }
        case .uniqueItems: return { [self] _, _ in try unique(value, source, path) }
        case .pattern(let program):
            return { [self] _, _ in if case .string(let text) = value { return try pattern(program, text, source, path) }; return true }
        }
    }
    private func type(_ types: [String], _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        if types.contains(value.kind) { return true }
        if types.contains("integer"), case .number(let n) = value { return try number(n.raw, source, path).isIntegral }
        return false
    }
    @inline(never) private func reference(_ target: Int, _ value: JsonValue, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        let result = try evaluate(target, value, path, depth + 1); produced = result.annotations; return result.valid
    }
    private func dynamicTarget(_ fallback: Int, _ anchor: String?, _ source: SourceLocation, _ path: String) throws -> Int {
        guard let graph = ValidationProgram.resourceContext else { throw failure(source, path, "dynamic instruction requires a checked resource graph") }
        // Pointer, empty-fragment and ordinary-anchor fallbacks never override.
        guard let anchor else { return fallback }
        // Only actually entered resources participate. In particular, the
        // fallback's resource has not been entered merely to perform this lookup.
        for resource in resourceStack {
            try spend(source, path)
            for (name, _, target) in graph.resources[resource].dynamicAnchors {
                try spend(source, path)
                if name.utf8.elementsEqual(anchor.utf8) { return target }
            }
        }
        return fallback
    }
    @inline(never) private func propertiesRule(_ fields: [(String, Int)], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for (name, target) in fields {
            if ValidationProgram.collectAnnotations { try spend(source, path) }
            if let child = object[name] {
                if !ValidationProgram.collectAnnotations { try spend(source, path) }
                let result = try evaluate(target, child, childPath(path, name), depth + 1)
                valid = result.valid && valid; mark(name, in: &produced)
            }
        }
        return valid
    }
    @inline(never) private func additional(_ declared: [String], _ target: Int, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        let known = Set(declared.map(JsonKey.init)); var valid = true
        for (name, child) in object.members {
            if ValidationProgram.collectAnnotations { try spend(source, path) }
            if known.contains(JsonKey(name)) { continue }
            if !ValidationProgram.collectAnnotations { try spend(source, path) }
            let result = try evaluate(target, child, childPath(path, name), depth + 1)
            valid = result.valid && valid; mark(name, in: &produced)
        }
        return valid
    }
    private func required(_ names: [String], _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for name in names { try spend(source, path); if object[name] == nil { valid = mismatch(source, childPath(path, name), "required property is absent") } }
        return valid
    }
    @inline(never) private func items(_ target: Int, _ start: Int, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .array(let array) = value, start < array.count else { return true }
        var valid = true
        for index in start..<array.count {
            try spend(source, path); let result = try evaluate(target, array[index], childPath(path, String(index)), depth + 1)
            valid = result.valid && valid; mark(index, in: &produced)
        }
        return valid
    }
    @inline(never) private func prefix(_ targets: [Int], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .array(let array) = value else { return true }
        var valid = true
        for (index, target) in targets.prefix(array.count).enumerated() {
            try spend(source, path); let result = try evaluate(target, array[index], childPath(path, String(index)), depth + 1)
            valid = result.valid && valid; mark(index, in: &produced)
        }
        return valid
    }
    @inline(never) private func composition(_ instruction: ValidationInstruction, _ targets: [Int], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        var count = 0; var passing: [Annotations] = []
        for target in targets {
            try spend(source, path)
            let result: Evaluated
            if case .allOf = instruction { result = try evaluate(target, value, path, depth + 1) }
            else { result = try trial(target, value, path, depth + 1) }
            if result.valid { count += 1; if ValidationProgram.collectAnnotations { passing.append(result.annotations) } }
        }
        let valid: Bool
        switch instruction { case .allOf: valid = count == targets.count; case .anyOf: valid = count > 0; default: valid = count == 1 }
        if valid { for annotation in passing { try merge(annotation, into: &produced, source: source, path: path) } }
        return valid
    }
    @inline(never) private func conditional(_ condition: Int, _ yes: Int?, _ no: Int?, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ local: inout Annotations, _ produced: inout Annotations) throws -> Bool {
        let tested = try trial(condition, value, path, depth + 1)
        if tested.valid { try merge(tested.annotations, into: &local, source: source, path: path) }
        guard let selected = tested.valid ? yes : no else { return true }
        let branch = try evaluate(selected, value, path, depth + 1); produced = branch.annotations; return branch.valid
    }
    private func dependentRequired(_ dependencies: [(String, [String])], _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for (trigger, names) in dependencies {
            try spend(source, path)
            if object[trigger] != nil {
                for name in names { try spend(source, path); if object[name] == nil { valid = mismatch(keyword(source, trigger), childPath(path, name), "dependent required property is absent") } }
            }
        }
        return valid
    }
    @inline(never) private func dependentSchemas(_ dependencies: [(String, Int)], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true; var passing: [Annotations] = []
        for (trigger, target) in dependencies {
            try spend(source, path)
            if object[trigger] != nil { let result = try evaluate(target, value, path, depth + 1); valid = result.valid && valid; if result.valid { passing.append(result.annotations) } }
        }
        if valid { for annotation in passing { try merge(annotation, into: &produced, source: source, path: path) } }
        return valid
    }
    @inline(never) private func contains(_ target: Int, _ minimum: String?, _ maximum: String?, _ value: JsonValue, _ node: SourceLocation, _ source: SourceLocation, _ path: String, _ depth: Int, _ local: inout Annotations) throws -> Bool {
        guard case .array(let array) = value else { return true }
        var count = 0; var matching = Annotations()
        for index in array.indices {
            try spend(source, path)
            if try trial(target, array[index], childPath(path, String(index)), depth + 1).valid { count += 1; mark(index, in: &matching) }
        }
        let actual = ExactDecimal(String(count)); let min = ExactDecimal(minimum ?? "1")
        let lower = actual.compare(min) >= 0
        let upper = maximum.map { actual.compare(ExactDecimal($0)) <= 0 } ?? true
        // contains itself annotates independently of adjacent min/max limits.
        if count > 0 || min.sign == 0 { try merge(matching, into: &local, source: source, path: path) }
        if !lower { _ = mismatch(minimum == nil ? source : keyword(node, "minContains"), path, "too few contains matches") }
        if !upper { _ = mismatch(keyword(node, "maxContains"), path, "too many contains matches") }
        return lower && upper
    }
    @inline(never) private func patternProperties(_ patterns: [(String, PatternProgram, Int)], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for (name, child) in object.members {
            try spend(source, path)
            for (_, program, target) in patterns {
                try spend(source, path)
                if try pattern(program, name, source, path) { let result = try evaluate(target, child, childPath(path, name), depth + 1); valid = result.valid && valid; mark(name, in: &produced) }
            }
        }
        return valid
    }
    @inline(never) private func additionalPatterns(_ declared: [String], _ target: Int, _ patterns: [(String, PatternProgram, Int)], _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        let known = Set(declared.map(JsonKey.init)); var valid = true
        members: for (name, child) in object.members {
            try spend(source, path)
            if known.contains(JsonKey(name)) { continue }
            for (_, program, _) in patterns { try spend(source, path); if try pattern(program, name, source, path) { continue members } }
            let result = try evaluate(target, child, childPath(path, name), depth + 1); valid = result.valid && valid; mark(name, in: &produced)
        }
        return valid
    }
    @inline(never) private func propertyNames(_ target: Int, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for name in object.keys { try spend(source, path); let result = try evaluate(target, .string(name), childPath(path, name), depth + 1); valid = result.valid && valid }
        return valid
    }
    @inline(never) private func unevaluatedProperties(_ target: Int, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ local: Annotations, _ produced: inout Annotations) throws -> Bool {
        guard case .object(let object) = value else { return true }
        var valid = true
        for (name, child) in object.members {
            try spend(source, path)
            if !local.properties.contains(JsonKey(name)) { let result = try evaluate(target, child, childPath(path, name), depth + 1); valid = result.valid && valid; mark(name, in: &produced) }
        }
        return valid
    }
    @inline(never) private func unevaluatedItems(_ target: Int, _ value: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int, _ local: Annotations, _ produced: inout Annotations) throws -> Bool {
        guard case .array(let array) = value else { return true }
        var valid = true
        for index in array.indices {
            try spend(source, path)
            if !local.items.contains(index) { let result = try evaluate(target, array[index], childPath(path, String(index)), depth + 1); valid = result.valid && valid; mark(index, in: &produced) }
        }
        return valid
    }
    private func bound(_ token: String, _ maximum: Bool, _ exclusive: Bool, _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        guard case .number(let n) = value else { return true }
        let order = try number(n.raw, source, path).compare(number(token, source, path))
        return maximum ? order < 0 || (order == 0 && !exclusive) : order > 0 || (order == 0 && !exclusive)
    }
    private func multiple(_ token: String, _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        guard case .number(let n) = value else { return true }
        return try number(n.raw, source, path).divisible(by: number(token, source, path)) { amount in
            guard amount <= self.numeric else { throw self.failure(source, path, "numeric work budget exhausted") }; self.numeric -= amount
        }
    }
    private func cardinality(_ token: String, _ maximum: Bool, _ target: String, _ value: JsonValue) -> Bool {
        let count: Int?
        switch (value, target) { case (.string(let s), "string"): count = s.unicodeScalars.count; case (.array(let a), "array"): count = a.count; case (.object(let o), "object"): count = o.count; default: count = nil }
        guard let count else { return true }
        let order = ExactDecimal(String(count)).compare(ExactDecimal(token)); return maximum ? order <= 0 : order >= 0
    }
    private func enumeration(_ candidates: [JsonValue], _ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        for candidate in candidates { try spend(source, path); if try equal(value, candidate, source, path, 0) { return true } }; return false
    }
    private func unique(_ value: JsonValue, _ source: SourceLocation, _ path: String) throws -> Bool {
        guard case .array(let array) = value else { return true }
        for index in array.indices { try spend(source, path); for previous in 0..<index { try spend(source, path); if try equal(array[index], array[previous], source, path, 0) { return false } } }; return true
    }
    private func equal(_ a: JsonValue, _ b: JsonValue, _ source: SourceLocation, _ path: String, _ depth: Int) throws -> Bool {
        guard equalities > 0, depth <= ValidationProgram.maxDepth else { throw failure(source, path, "equality budget exhausted") }
        equalities -= 1
        switch (a, b) {
        case (.null, .null): return true
        case let (.bool(a), .bool(b)): return a == b
        case let (.string(a), .string(b)): return a.utf8.elementsEqual(b.utf8)
        case let (.number(a), .number(b)): return try number(a.raw, source, path).compare(number(b.raw, source, path)) == 0
        case let (.array(a), .array(b)):
            if a.count != b.count { return false }
            for (x, y) in zip(a, b) { if try !equal(x, y, source, path, depth + 1) { return false } }
            return true
        case let (.object(a), .object(b)):
            if a.count != b.count { return false }
            for (key, value) in a.members {
                guard let other = b[key] else { return false }
                if try !equal(value, other, source, path, depth + 1) { return false }
            }
            return true
        default: return false
        }
    }
    private func pattern(_ program: PatternProgram, _ text: String, _ source: SourceLocation, _ path: String) throws -> Bool {
        try spend(source, path)
        let scalars = text.unicodeScalars
        var cursor = scalars.startIndex
        var position = 0
        var seeds: [Int] = []
        while true {
            let atEnd = cursor == scalars.endIndex
            var pending = seeds + [program.start]
            var seen: Set<Int> = []
            var consuming: [Int] = []
            while let index = pending.popLast() {
                try spend(source, path)
                if !seen.insert(index).inserted { continue }
                switch program.states[index] {
                case .match: return true
                case .split(let a, let b): pending.append(contentsOf: [b, a])
                case .jump(let target): pending.append(target)
                case .start(let target) where position == 0: pending.append(target)
                case .end(let target) where atEnd: pending.append(target)
                case .char: consuming.append(index)
                default: break
                }
            }
            if atEnd { return false }
            let scalar = scalars[cursor].value
            seeds = []
            for index in consuming {
                if case .char(let ranges, let target) = program.states[index] {
                    for (low, high) in ranges {
                        try spend(source, path)
                        if scalar < low { break }
                        if scalar <= high { seeds.append(target); break }
                    }
                }
            }
            scalars.formIndex(after: &cursor)
            position += 1
        }
    }
}
