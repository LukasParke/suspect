// Appended to the unchanged public v1 codec/presence/budget declarations only
// when the checked program declares the scoped v2 profile.

internal object ValidationProgram {
    val compiled: ScopedProgram = run {
        val bytes = ValidationProgram::class.java.getResourceAsStream("validation.json")?.use { it.readNBytes(Json.MAX_PROGRAM_BYTES + 1) }
            ?: error("generated validation program is missing")
        ScopedProgram(Json.parseProgram(bytes) as JsonObject)
    }
    val nodes: List<JsonObject> get() = compiled.nodes
    val roots: Map<SourceLocation, Int> get() = compiled.roots
    val limits: JsonObject get() = compiled.limits
}

internal fun JsonObject.text(key: String): String = (values.getValue(key) as JsonString).value
internal fun JsonObject.flag(key: String): Boolean = (values.getValue(key) as JsonBoolean).value
internal fun JsonObject.number(key: String): Int = (values.getValue(key) as JsonNumber).toLongExact().also { require(it in 0..Int.MAX_VALUE.toLong()) }.toInt()
internal fun JsonObject.obj(key: String): JsonObject = values.getValue(key) as JsonObject
internal fun JsonObject.array(key: String): List<JsonValue> = (values.getValue(key) as JsonArray).values
internal fun JsonObject.source(): SourceLocation = obj("source").let { SourceLocation(it.text("document"), it.text("pointer")) }
internal fun scopedChild(source: SourceLocation, key: String): SourceLocation = SourceLocation(source.document, childPath(source.pointer, key))
internal fun JsonObject.scopedTarget(key: String): Int? = if (values.getValue(key) === JsonNull) null else number(key)
internal fun scopedIndex(value: JsonValue): Int = (value as JsonNumber).toLongExact().also { require(it in 0..Int.MAX_VALUE.toLong()) }.toInt()

internal val scopedKeyOrder: Comparator<String> = Comparator { left, right ->
    var a = 0
    var b = 0
    var order = 0
    while (a < left.length && b < right.length) {
        val x = left.codePointAt(a)
        val y = right.codePointAt(b)
        if (x != y) { order = x.compareTo(y); break }
        a += Character.charCount(x)
        b += Character.charCount(y)
    }
    if (order != 0) order else if (a == left.length && b == right.length) 0 else if (a == left.length) -1 else 1
}

/** Checked in Rust before emission and structurally rechecked on native load. */
internal class ScopedProgram(value: JsonObject) {
    val nodes: List<JsonObject> = value.array("nodes").map { it as JsonObject }
    val limits: JsonObject = value.obj("limits")
    val roots: Map<SourceLocation, Int>
    init {
        require(value.text("version") == "suspect.validation.experimental.v2" && value.text("profile") == "oas31-jsonschema202012-static-applicators") { "unknown scoped validation version/profile" }
        require(nodes.size <= 16_384)
        require(limits.number("maxDepth") <= 128 && limits.number("maxNumberBytes") <= 4096)
        for (name in listOf("maxEvaluationSteps", "maxEqualitySteps", "maxErrors")) require(limits.number(name) <= 100000)
        val identities = nodes.map { it.source().also(::location) }
        require(identities.toSet().size == identities.size) { "duplicate schema identity" }
        val rootEntries = value.array("roots").map { it as JsonObject }
        require(rootEntries.map { it.source() }.toSet().size == rootEntries.size)
        roots = rootEntries.associate { root ->
            val source = root.source().also(::location)
            val target = root.number("target")
            require(target in nodes.indices && nodes[target].source() == source)
            source to target
        }
        for (node in nodes) checkNode(node)
    }
    private fun location(source: SourceLocation) {
        val uri = java.net.URI(source.document)
        require(uri.isAbsolute && uri.rawFragment == null)
        require(source.pointer.isEmpty() || source.pointer.startsWith('/'))
        var index = 0
        while (index < source.pointer.length) {
            if (source.pointer[index++] == '~') require(index < source.pointer.length && source.pointer[index++] in "01")
        }
    }
    private fun strings(value: List<JsonValue>): List<String> = value.map { (it as JsonString).value }.also { require(it.toSet().size == it.size) }
    private fun target(index: Int, expected: SourceLocation? = null) {
        require(index in nodes.indices) { "compiled target outside graph" }
        if (expected != null) require(nodes[index].source() == expected) { "compiled target/source mismatch" }
    }
    private fun numeric(token: String, count: Boolean = false) {
        require(token.length <= limits.number("maxNumberBytes")) { "compiled numeric operand exceeds limit" }
        val number = JsonNumber.parse(token)
        if (count) require(number.isInteger() && number.exact.sign >= 0) { "count must be a nonnegative mathematical integer" }
    }
    private fun literal(value: JsonValue) {
        when (value) {
            is JsonNumber -> numeric(value.token)
            is JsonArray -> value.values.forEach(::literal)
            is JsonObject -> value.values.values.forEach(::literal)
            else -> Unit
        }
    }
    private fun pattern(program: JsonObject) {
        require(program.text("version") == "suspect.pattern.experimental.v1")
        val states = program.array("states").map { it as JsonObject }
        require(states.isNotEmpty() && states.size <= 4096 && program.number("start") in states.indices)
        var ranges = 0
        fun to(index: Int) { require(index in states.indices) }
        for (state in states) when (state.text("op")) {
            "match" -> Unit
            "split" -> { to(state.number("first")); to(state.number("second")) }
            "jump", "start", "end" -> to(state.number("target"))
            "char" -> {
                to(state.number("target"))
                var previous = -2
                val pairs = state.array("ranges")
                ranges += pairs.size
                require(pairs.size <= 4096 && ranges <= 65536)
                for (raw in pairs) {
                    val pair = (raw as JsonArray).values
                    require(pair.size == 2)
                    val a = (pair[0] as JsonNumber).toLongExact()
                    val b = (pair[1] as JsonNumber).toLongExact()
                    require(a in 0..0x10ffff && b in a..0x10ffff && a > previous.toLong() + 1)
                    previous = b.toInt()
                }
            }
            else -> error("unknown pattern state")
        }
    }
    private fun checkNode(node: JsonObject) {
        val source = node.source()
        val checks = node.array("checks").map { it as JsonObject }
        val locations = mutableSetOf<SourceLocation>()
        var names = emptyList<String>()
        var declared: List<String>? = null
        var patternCount = 0
        var additionalPatterns = false
        var hasAdditional = false
        var tail = false
        var prefix = 0
        var start: Int? = null
        for (check in checks) {
            val at = check.source().also(::location)
            require(locations.add(at))
            val op = check.text("op")
            val unevaluated = op == "unevaluatedProperties" || op == "unevaluatedItems"
            require(!tail || unevaluated) { "unevaluated checks must be last" }
            tail = tail || unevaluated
            val key = when (op) {
                "always" -> null
                "ref" -> "\$ref"
                "bound" -> if (check.flag("maximum")) { if (check.flag("exclusive")) "exclusiveMaximum" else "maximum" } else { if (check.flag("exclusive")) "exclusiveMinimum" else "minimum" }
                "count" -> (if (check.flag("maximum")) "max" else "min") + when (check.text("target")) { "array" -> "Items"; "object" -> "Properties"; "string" -> "Length"; else -> error("invalid count target") }
                "additionalPropertiesWithPatterns" -> "additionalProperties"
                else -> op
            }
            val normalizedBound = op == "bound" && check.flag("exclusive") && at == scopedChild(source, if (check.flag("maximum")) "maximum" else "minimum")
            require(at == (if (key == null) source else scopedChild(source, key)) || normalizedBound) { "compiled keyword/source mismatch" }
            when (op) {
                "always" -> { check.flag("value"); require(checks.size == 1) }
                "type" -> { val types = strings(check.array("types")); require(types.isNotEmpty() && types.all { it in listOf("null", "boolean", "string", "number", "integer", "array", "object") }) }
                "ref" -> target(check.number("target"))
                "properties" -> {
                    val fields = check.array("properties").map { it as JsonObject }
                    names = fields.map { it.text("name") }; require(names.toSet().size == names.size)
                    for (field in fields) target(field.number("target"), scopedChild(at, field.text("name")))
                }
                "additionalProperties", "additionalPropertiesWithPatterns" -> {
                    declared = strings(check.array("declared")); hasAdditional = true
                    additionalPatterns = op == "additionalPropertiesWithPatterns"
                    target(check.number("target"), at)
                }
                "required" -> strings(check.array("names"))
                "items" -> { start = check.number("start"); target(check.number("target"), at) }
                "prefixItems", "allOf", "anyOf", "oneOf" -> {
                    val targets = check.array("targets"); require(targets.isNotEmpty())
                    if (op == "prefixItems") prefix = targets.size
                    for ((i, id) in targets.withIndex()) target(scopedIndex(id), scopedChild(at, i.toString()))
                }
                "not", "propertyNames", "unevaluatedProperties", "unevaluatedItems" -> target(check.number("target"), at)
                "if" -> {
                    target(check.number("condition"), at)
                    check.scopedTarget("thenTarget")?.let { target(it, scopedChild(source, "then")) }
                    check.scopedTarget("elseTarget")?.let { target(it, scopedChild(source, "else")) }
                }
                "dependentRequired" -> {
                    val triggers = mutableSetOf<String>()
                    for (raw in check.array("dependencies")) {
                        val pair = (raw as JsonArray).values; require(pair.size == 2)
                        require(triggers.add((pair[0] as JsonString).value)); strings((pair[1] as JsonArray).values)
                    }
                }
                "dependentSchemas" -> {
                    val triggers = mutableSetOf<String>()
                    for (raw in check.array("dependencies")) {
                        val entry = raw as JsonObject; val name = entry.text("name")
                        require(triggers.add(name)); target(entry.number("target"), scopedChild(at, name))
                    }
                }
                "contains" -> {
                    target(check.number("target"), at)
                    for (keyName in listOf("minimum", "maximum")) if (check.values.getValue(keyName) !== JsonNull) numeric(check.text(keyName), true)
                }
                "patternProperties" -> {
                    val patterns = check.array("patterns"); patternCount = patterns.size
                    val distinct = mutableSetOf<String>()
                    for (raw in patterns) {
                        val triple = (raw as JsonArray).values; require(triple.size == 3)
                        val name = (triple[0] as JsonString).value; require(distinct.add(name))
                        pattern(triple[1] as JsonObject)
                        target(scopedIndex(triple[2]), scopedChild(at, name))
                    }
                }
                "bound", "multipleOf" -> { numeric(check.text("value")); if (op == "multipleOf") require(JsonNumber.parse(check.text("value")).exact.sign > 0) }
                "count" -> numeric(check.text("value"), true)
                "const" -> literal(check.values.getValue("value"))
                "enum" -> { require(check.array("values").isNotEmpty()); check.array("values").forEach(::literal) }
                "uniqueItems" -> Unit
                "pattern" -> pattern(check.obj("program"))
                else -> error("unknown scoped instruction")
            }
        }
        require(declared == null || declared.toSet() == names.toSet())
        require(!hasAdditional || additionalPatterns == (patternCount != 0))
        require(start == null || start == prefix)
    }
}

internal class ScopedMarks {
    val properties = java.util.TreeSet<String>(scopedKeyOrder)
    val items = java.util.TreeSet<Int>()
}
internal class ScopedResult(val valid: Boolean, val marks: ScopedMarks)

internal class ValidationSession(
    limits: CodecLimits,
    private val checkpoint: () -> Unit,
    private val program: ScopedProgram = ValidationProgram.compiled,
) {
    private val programLimits = program.limits
    private var steps = minOf(programLimits.number("maxEvaluationSteps"), limits.maxEvaluationSteps)
    private var equalitySteps = minOf(programLimits.number("maxEqualitySteps"), limits.maxEqualitySteps)
    private var numericSteps = limits.maxNumericSteps
    private var textBytes = limits.maxValidationBytes
    private val maxDepth = minOf(programLimits.number("maxDepth"), limits.json.maxDepth)
    private val maxErrors = programLimits.number("maxErrors")
    private val maxNumber = minOf(programLimits.number("maxNumberBytes"), limits.json.maxNumberBytes)
    private val active = java.util.IdentityHashMap<JsonValue, MutableSet<Int>>()
    private val numbers = java.util.IdentityHashMap<JsonNumber, ExactDecimal>()
    private var findings = mutableListOf<ValidationFinding>()

    fun validate(target: Int, value: JsonValue, path: String) {
        if (!eval(target, value, path, 0).valid) throw ValidationException(findings.toList())
    }
    fun matches(target: Int, value: JsonValue, path: String): Boolean = trial(target, value, path, 0).valid
    private fun fail(source: SourceLocation, path: String, message: String): Nothing = throw EvaluationException(ValidationFinding(source, path, message))
    private fun step(source: SourceLocation, path: String) {
        checkpoint()
        if (steps-- <= 0) fail(source, path, "schema evaluation step limit exceeded")
    }
    private fun numeric(source: SourceLocation, path: String, amount: Int) {
        checkpoint()
        if (amount > numericSteps) fail(source, path, "exact arithmetic work limit exceeded")
        numericSteps -= amount
    }
    private fun text(source: SourceLocation, path: String, value: String) {
        checkpoint()
        val amount = try { Json.utf8Size(value, textBytes, checkpoint) }
        catch (error: JsonException) { if (error.kind == JsonErrorKind.RESOURCE_LIMIT) fail(source, path, "assertion text work limit exceeded") else throw error }
        textBytes -= amount
    }
    private fun reject(source: SourceLocation, path: String, message: String): Boolean {
        if (maxErrors == 0 || findings.size < maxErrors) findings.add(ValidationFinding(source, path, message))
        return false
    }
    private fun number(value: JsonNumber, source: SourceLocation, path: String): ExactDecimal {
        if (value.token.length > maxNumber) fail(source, path, "exact number byte limit exceeded")
        return numbers[value] ?: run {
            numeric(source, path, 1 + value.token.length / 16)
            value.exact.also { numbers[value] = it }
        }
    }
    private fun merge(into: ScopedMarks, other: ScopedMarks, source: SourceLocation, path: String) {
        for (key in other.properties) { step(source, path); into.properties.add(key) }
        for (index in other.items) { step(source, path); into.items.add(index) }
    }
    private fun trial(target: Int, value: JsonValue, path: String, depth: Int): ScopedResult {
        val saved = findings
        findings = mutableListOf()
        try { return eval(target, value, path, depth) }
        finally { findings = saved }
    }
    private fun eval(target: Int, value: JsonValue, path: String, depth: Int): ScopedResult {
        val node = program.nodes.getOrNull(target) ?: fail(SourceLocation("", ""), path, "invalid generated schema target")
        val source = node.source()
        step(source, path)
        if (depth >= maxDepth) fail(source, path, "schema evaluation depth limit exceeded")
        val identities = active.getOrPut(value) { mutableSetOf() }
        if (!identities.add(target)) fail(source, path, "recursive schema made no instance progress")
        try {
            val local = ScopedMarks()
            var valid = true
            for (raw in node.array("checks")) {
                val check = raw as JsonObject
                step(check.source(), path)
                if (!apply(node, check, value, path, depth, local)) valid = false
            }
            return ScopedResult(valid, if (valid) local else ScopedMarks())
        } finally {
            identities.remove(target)
            if (identities.isEmpty()) active.remove(value)
        }
    }
    private fun apply(node: JsonObject, check: JsonObject, value: JsonValue, path: String, depth: Int, local: ScopedMarks): Boolean {
        val at = check.source()
        var produced = ScopedMarks()
        val okay = when (val op = check.text("op")) {
            "always" -> check.flag("value") || reject(at, path, "false schema rejects every value")
            "type" -> {
                val types = check.array("types").map { (it as JsonString).value }
                val accepted = when (value) {
                    JsonNull -> "null" in types
                    is JsonBoolean -> "boolean" in types
                    is JsonNumber -> "number" in types || "integer" in types && number(value, at, path).integral
                    is JsonString -> "string" in types
                    is JsonArray -> "array" in types
                    is JsonObject -> "object" in types
                }
                accepted || reject(at, path, "instance does not match the declared type")
            }
            "ref" -> eval(check.number("target"), value, path, depth + 1).let { produced = it.marks; it.valid }
            "properties" -> {
                var okay = true
                if (value is JsonObject) for (raw in check.array("properties")) {
                    step(at, path)
                    val property = raw as JsonObject
                    val key = property.text("name")
                    value.values[key]?.let {
                        if (!eval(property.number("target"), it, childPath(path, key), depth + 1).valid) okay = false
                        produced.properties.add(key)
                    }
                }
                okay
            }
            "required" -> {
                var okay = true
                if (value is JsonObject) for (raw in check.array("names")) {
                    step(at, path)
                    if ((raw as JsonString).value !in value.values) { reject(at, path, "required property is absent"); okay = false }
                }
                okay
            }
            "items", "prefixItems" -> {
                var okay = true
                if (value is JsonArray) {
                    val targets = if (op == "prefixItems") check.array("targets") else null
                    val start = if (targets == null) check.number("start") else 0
                    val end = if (targets == null) value.values.size else minOf(value.values.size, targets.size)
                    for (index in start until end) {
                        step(at, path)
                        val target = targets?.get(index)?.let(::scopedIndex) ?: check.number("target")
                        if (!eval(target, value.values[index], childPath(path, index.toString()), depth + 1).valid) okay = false
                        produced.items.add(index)
                    }
                }
                okay
            }
            "allOf", "anyOf", "oneOf" -> {
                var matches = 0
                val passing = mutableListOf<ScopedMarks>()
                val targets = check.array("targets")
                for (raw in targets) {
                    step(at, path)
                    val target = scopedIndex(raw)
                    val result = if (op == "allOf") eval(target, value, path, depth + 1) else trial(target, value, path, depth + 1)
                    if (result.valid) { matches++; passing.add(result.marks) }
                }
                val valid = when (op) { "allOf" -> matches == targets.size; "anyOf" -> matches > 0; else -> matches == 1 }
                if (valid) passing.forEach { merge(produced, it, at, path) }
                valid || reject(at, path, "composition matched $matches alternatives")
            }
            "not" -> !trial(check.number("target"), value, path, depth + 1).valid || reject(at, path, "instance matches the negated schema")
            "if" -> {
                val condition = trial(check.number("condition"), value, path, depth + 1)
                val selected = check.scopedTarget(if (condition.valid) "thenTarget" else "elseTarget")
                if (condition.valid) merge(local, condition.marks, at, path)
                if (selected == null) true else eval(selected, value, path, depth + 1).let { produced = it.marks; it.valid }
            }
            "dependentRequired" -> {
                var okay = true
                if (value is JsonObject) for (raw in check.array("dependencies")) {
                    step(at, path)
                    val pair = (raw as JsonArray).values
                    val trigger = (pair[0] as JsonString).value
                    if (value.values.containsKey(trigger)) for (name in (pair[1] as JsonArray).values) {
                        step(at, path)
                        if (!value.values.containsKey((name as JsonString).value)) { reject(scopedChild(at, trigger), path, "dependent required property is absent"); okay = false }
                    }
                }
                okay
            }
            "dependentSchemas" -> {
                var okay = true
                val passing = mutableListOf<ScopedMarks>()
                if (value is JsonObject) for (raw in check.array("dependencies")) {
                    step(at, path)
                    val dependency = raw as JsonObject
                    if (value.values.containsKey(dependency.text("name"))) {
                        val result = eval(dependency.number("target"), value, path, depth + 1)
                        if (result.valid) passing.add(result.marks) else okay = false
                    }
                }
                if (okay) passing.forEach { merge(produced, it, at, path) }
                okay
            }
            "contains" -> {
                if (value !is JsonArray) true else {
                    var matches = 0
                    for ((index, item) in value.values.withIndex()) {
                        step(at, path)
                        if (trial(check.number("target"), item, childPath(path, index.toString()), depth + 1).valid) { matches++; produced.items.add(index) }
                    }
                    val minimum = (check.values["minimum"] as? JsonString)?.value?.let { JsonNumber.parse(it).exact }
                    val maximum = (check.values["maximum"] as? JsonString)?.value?.let { JsonNumber.parse(it).exact }
                    if (matches > 0 || minimum?.sign == 0) { merge(local, produced, at, path); produced = ScopedMarks() }
                    val count = JsonNumber.of(matches.toLong()).exact
                    val lower = minimum?.let { count >= it } ?: (matches >= 1)
                    val upper = maximum?.let { count <= it } ?: true
                    if (!lower) reject(if (minimum == null) at else scopedChild(node.source(), "minContains"), path, "too few contains matches")
                    if (!upper) reject(scopedChild(node.source(), "maxContains"), path, "too many contains matches")
                    lower && upper
                }
            }
            "patternProperties" -> {
                var okay = true
                if (value is JsonObject) for (name in value.values.keys.sortedWith(scopedKeyOrder)) {
                    step(at, path)
                    for (raw in check.array("patterns")) {
                        step(at, path)
                        val triple = (raw as JsonArray).values
                        if (pattern(triple[1] as JsonObject, name, at, path)) {
                            if (!eval(scopedIndex(triple[2]), value.values.getValue(name), childPath(path, name), depth + 1).valid) okay = false
                            produced.properties.add(name)
                        }
                    }
                }
                okay
            }
            "additionalProperties", "additionalPropertiesWithPatterns" -> {
                var okay = true
                val declared = check.array("declared").map { (it as JsonString).value }.toSet()
                val patterns = if (op == "additionalPropertiesWithPatterns") node.array("checks").map { it as JsonObject }.first { it.text("op") == "patternProperties" }.array("patterns") else emptyList()
                if (value is JsonObject) for (name in value.values.keys.sortedWith(scopedKeyOrder)) {
                    step(at, path)
                    if (name in declared) continue
                    var matched = false
                    for (raw in patterns) {
                        step(at, path)
                        if (pattern((raw as JsonArray).values[1] as JsonObject, name, at, path)) { matched = true; break }
                    }
                    if (!matched) {
                        if (!eval(check.number("target"), value.values.getValue(name), childPath(path, name), depth + 1).valid) okay = false
                        produced.properties.add(name)
                    }
                }
                okay
            }
            "propertyNames" -> {
                var okay = true
                if (value is JsonObject) for (name in value.values.keys.sortedWith(scopedKeyOrder)) {
                    step(at, path)
                    if (!eval(check.number("target"), JsonString(name), childPath(path, name), depth + 1).valid) okay = false
                }
                okay
            }
            "unevaluatedProperties" -> {
                var okay = true
                if (value is JsonObject) for (name in value.values.keys.sortedWith(scopedKeyOrder)) {
                    step(at, path)
                    if (name !in local.properties) {
                        if (!eval(check.number("target"), value.values.getValue(name), childPath(path, name), depth + 1).valid) okay = false
                        produced.properties.add(name)
                    }
                }
                okay
            }
            "unevaluatedItems" -> {
                var okay = true
                if (value is JsonArray) for ((index, item) in value.values.withIndex()) {
                    step(at, path)
                    if (index !in local.items) {
                        if (!eval(check.number("target"), item, childPath(path, index.toString()), depth + 1).valid) okay = false
                        produced.items.add(index)
                    }
                }
                okay
            }
            "bound" -> {
                if (value !is JsonNumber) true else {
                    val order = number(value, at, path).compareTo(JsonNumber.parse(check.text("value")).exact)
                    val valid = if (check.flag("maximum")) order < 0 || order == 0 && !check.flag("exclusive") else order > 0 || order == 0 && !check.flag("exclusive")
                    valid || reject(at, path, "number violates its exact bound")
                }
            }
            "multipleOf" -> value !is JsonNumber || number(value, at, path).multipleOf(JsonNumber.parse(check.text("value")).exact) { numeric(at, path, it) } || reject(at, path, "number is not an exact multiple")
            "count" -> {
                val count = when (check.text("target")) {
                    "string" -> (value as? JsonString)?.value?.let { text(at, path, it); it.codePointCount(0, it.length) }
                    "array" -> (value as? JsonArray)?.values?.size
                    "object" -> (value as? JsonObject)?.values?.size
                    else -> fail(at, path, "unknown cardinality target")
                }
                if (count == null) true else {
                    val order = JsonNumber.of(count.toLong()).exact.compareTo(JsonNumber.parse(check.text("value")).exact)
                    (if (check.flag("maximum")) order <= 0 else order >= 0) || reject(at, path, "instance violates cardinality")
                }
            }
            "const" -> equal(value, check.values.getValue("value"), at, path, 0) || reject(at, path, "instance differs from const")
            "enum" -> {
                var found = false
                for (candidate in check.array("values")) { step(at, path); if (equal(value, candidate, at, path, 0)) { found = true; break } }
                found || reject(at, path, "instance is not an enum member")
            }
            "uniqueItems" -> {
                var unique = true
                if (value is JsonArray) outer@ for (i in value.values.indices) {
                    step(at, path)
                    for (j in 0 until i) {
                        step(at, path)
                        if (equal(value.values[i], value.values[j], at, path, 0)) { unique = false; break@outer }
                    }
                }
                unique || reject(at, path, "array items are not unique")
            }
            "pattern" -> value !is JsonString || pattern(check.obj("program"), value.value, at, path) || reject(at, path, "string does not match the portable pattern")
            else -> fail(at, path, "unsupported scoped instruction")
        }
        if (okay) merge(local, produced, at, path)
        return okay
    }
    private fun equal(a: JsonValue, b: JsonValue, source: SourceLocation, path: String, depth: Int): Boolean {
        val pending = java.util.ArrayDeque<kotlin.Triple<JsonValue, JsonValue, Int>>()
        pending.addLast(kotlin.Triple(a, b, depth))
        while (pending.isNotEmpty()) {
            checkpoint()
            val (left, right, level) = pending.removeLast()
            if (equalitySteps-- <= 0 || level > maxDepth) fail(source, path, "structural equality limit exceeded")
            val same = when {
                left is JsonNumber && right is JsonNumber -> number(left, source, path).compareTo(number(right, source, path)) == 0
                left is JsonString && right is JsonString -> { text(source, path, left.value); text(source, path, right.value); left.value == right.value }
                left is JsonArray && right is JsonArray -> {
                    if (left.values.size != right.values.size) return false
                    if (left.values.size.toLong() + pending.size > equalitySteps) fail(source, path, "structural equality limit exceeded")
                    for (index in left.values.indices.reversed()) pending.addLast(kotlin.Triple(left.values[index], right.values[index], level + 1))
                    true
                }
                left is JsonObject && right is JsonObject -> {
                    if (left.values.size != right.values.size) return false
                    if (left.values.size.toLong() + pending.size > equalitySteps) fail(source, path, "structural equality limit exceeded")
                    for (key in left.values.keys.sortedWith(scopedKeyOrder).asReversed()) {
                        text(source, path, key)
                        val other = right.values[key] ?: return false
                        pending.addLast(kotlin.Triple(left.values.getValue(key), other, level + 1))
                    }
                    true
                }
                else -> left == right
            }
            if (!same) return false
        }
        return true
    }
    private fun pattern(program: JsonObject, input: String, source: SourceLocation, path: String): Boolean {
        val states = program.array("states").map { it as JsonObject }
        val seen = IntArray(states.size)
        var seeds = mutableListOf<Int>()
        var offset = 0
        var epoch = 0
        while (true) {
            step(source, path)
            epoch++
            val stack = mutableListOf<Int>()
            val activeStates = mutableListOf<Int>()
            fun enqueue(index: Int) {
                step(source, path)
                if (seen[index] != epoch) { seen[index] = epoch; stack.add(index) }
            }
            enqueue(program.number("start"))
            for (seed in seeds) enqueue(seed)
            while (stack.isNotEmpty()) {
                step(source, path)
                val index = stack.removeAt(stack.lastIndex)
                val state = states[index]
                when (state.text("op")) {
                    "match" -> return true
                    "split" -> { enqueue(state.number("second")); enqueue(state.number("first")) }
                    "jump" -> enqueue(state.number("target"))
                    "start" -> if (offset == 0) enqueue(state.number("target"))
                    "end" -> if (offset == input.length) enqueue(state.number("target"))
                    "char" -> activeStates.add(index)
                    else -> fail(source, path, "unknown compiled pattern state")
                }
            }
            if (offset == input.length) return false
            val scalar = input.codePointAt(offset)
            offset += Character.charCount(scalar)
            val next = mutableListOf<Int>()
            for (index in activeStates) {
                val state = states[index]
                for (range in state.array("ranges")) {
                    step(source, path)
                    val bounds = (range as JsonArray).values
                    if (scalar < (bounds[0] as JsonNumber).toLongExact()) break
                    if (scalar <= (bounds[1] as JsonNumber).toLongExact()) { next.add(state.number("target")); break }
                }
            }
            seeds = next
        }
    }
}
