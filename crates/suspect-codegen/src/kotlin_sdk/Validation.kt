package __PACKAGE__

/** Optional presence is independent of Kotlin nullability.
 * `Presence<String>` cannot contain null; `Presence<String?>` can.
 */
public sealed interface Presence<out T> {
    /** Omit this member completely. */
    public data object Absent : Presence<Nothing>
    /** Include this member, including explicit null when T is nullable.
     * @property value Present value.
     */
    public data class Present<T>(/** Present value; nullable only when T is nullable. */ public val value: T) : Presence<T>
}

/** Original contract identity, independent of generated paths.
 * @property document Canonical document URI.
 * @property pointer Escaped RFC 6901 source pointer.
 */
public data class SourceLocation(/** Canonical document URI. */ public val document: String, /** Escaped RFC 6901 source pointer. */ public val pointer: String)

/** A source assertion mismatch. Structured paths retain exact Unicode keys.
 * @property source Original schema/keyword identity.
 * @property instancePath Escaped RFC 6901 instance pointer.
 * @property message Explanation without instance values.
 */
public data class ValidationFinding(/** Original schema/keyword identity. */ public val source: SourceLocation, /** Exact escaped instance pointer. */ public val instancePath: String, /** Explanation excluding instance values. */ public val message: String)

internal fun diagnosticText(value: String): String = buildString {
    for (c in value.take(256)) {
        when {
            c < ' ' || c == '\u007F' || c in '\u2028'..'\u202E' || c in '\u2066'..'\u2069' -> append("\\u" + c.code.toString(16).padStart(4, '0'))
            else -> append(c)
        }
    }
    if (value.length > 256) append("…")
}
internal fun findingText(finding: ValidationFinding): String =
    "${diagnosticText(finding.message)} at ${diagnosticText(finding.source.document)}#${diagnosticText(finding.source.pointer)} (instance ${diagnosticText(finding.instancePath)})"

/** Completed validation rejected a value.
 * @property findings Bounded, source-linked mismatches.
 */
public class ValidationException(/** Bounded, source-linked mismatches. */ public val findings: List<ValidationFinding>) : IllegalArgumentException(
    findings.firstOrNull()?.let(::findingText) ?: "schema validation failed"
)

/** Evaluation could not complete. Logical alternatives cannot suppress this.
 * @property finding Source-linked recursion, arithmetic or work failure.
 */
public class EvaluationException(/** The source-linked evaluation failure. */ public val finding: ValidationFinding) : IllegalStateException(findingText(finding))

/** Per-codec finite resource policy. Counters are shared across all native
 * traversal, selected-branch checks, parent checks and logical trials in a call.
 * Callers may lower ceilings, including to zero; source constraints are unchanged.
 * @property json JSON parsing/writing ceilings.
 * @property maxModelSteps Native traversal visits, at most 100000.
 * @property maxModelBytes Bytes examined/copied by native conversion, at most 16 MiB.
 * @property maxEvaluationSteps Schema/keyword/collection/NFA visits, at most 100000.
 * @property maxEqualitySteps Structural equality pairs, at most 100000.
 * @property maxNumericSteps Bounded exact arithmetic work, at most 100000.
 * @property maxValidationBytes Text scanned/compared by assertions, at most 16 MiB.
 */
public data class CodecLimits(
    /** JSON representation ceilings. */
    public val json: JsonLimits = JsonLimits(),
    /** Shared native traversal visits. */
    public val maxModelSteps: Int = 100_000,
    /** Shared native conversion UTF-8 bytes. */
    public val maxModelBytes: Int = 16 * 1024 * 1024,
    /** Shared schema/keyword/collection/NFA visits. */
    public val maxEvaluationSteps: Int = 100_000,
    /** Shared structural equality pairs. */
    public val maxEqualitySteps: Int = 100_000,
    /** Shared exact arithmetic work. */
    public val maxNumericSteps: Int = 100_000,
    /** Shared UTF-8 text examined by assertions. */
    public val maxValidationBytes: Int = 16 * 1024 * 1024,
) {
    init {
        require(listOf(maxModelSteps, maxEvaluationSteps, maxEqualitySteps, maxNumericSteps).all { it in 0..100000 }
            && maxModelBytes in 0..16777216 && maxValidationBytes in 0..16777216) { "codec limits must be within the finite profile" }
    }
}

/** A source-bound native codec; encode revalidates mutable values and data-class copies.
 * @property source Exact source schema identity.
 */
public class ModelCodec<T> internal constructor(
    private val target: Int,
    /** Exact source schema identity. */
    public val source: SourceLocation,
    private val read: (JsonValue, ModelBudget, String) -> T,
    private val write: (T, ModelBudget, String) -> JsonValue,
) {
    /** Parse, validate the complete schema, then construct the native value. */
    public fun decode(text: String, limits: CodecLimits = CodecLimits()): T =
        decodeChecked(Json.parse(text, limits.json), limits) {}
    /** Strict UTF-8 decode with exact numbers and complete source validation. */
    public fun decode(bytes: ByteArray, limits: CodecLimits = CodecLimits()): T =
        decodeChecked(Json.parse(bytes, limits.json), limits) {}
    /** Snapshot a native JSON tree, validate it and construct the model. */
    public fun decodeJson(value: JsonValue, limits: CodecLimits = CodecLimits()): T = decodeChecked(value, limits) {}
    /** Reconstruct and validate an independent JSON tree, including selected union arms. */
    public fun encodeJson(value: T, limits: CodecLimits = CodecLimits()): JsonValue = encodeJsonChecked(value, limits) {}
    /** Emit deterministic JSON after native conversion and complete validation. */
    public fun encode(value: T, limits: CodecLimits = CodecLimits()): String = encodeChecked(value, limits) {}

    internal fun encodeUsing(value: T, budget: ModelBudget, path: String): JsonValue {
        val result = write(value, budget, path)
        budget.validation.validate(target, result, path)
        return budget.representation(result)
    }
    internal fun decodeUsing(value: JsonValue, budget: ModelBudget, path: String): T {
        val snapshot = budget.json(value, source, path)
        budget.representation(snapshot)
        budget.validation.validate(target, snapshot, path)
        return read(snapshot, budget, path)
    }

    internal fun decodeChecked(value: JsonValue, limits: CodecLimits, checkpoint: () -> Unit): T {
        val budget = ModelBudget(limits, checkpoint)
        val snapshot = budget.json(value, source, "")
        Json.stringifyChecked(snapshot, limits.json, checkpoint)
        budget.validation.validate(target, snapshot, "")
        return read(snapshot, budget, "").also { checkpoint() }
    }
    internal fun encodeJsonChecked(value: T, limits: CodecLimits, checkpoint: () -> Unit): JsonValue {
        val budget = ModelBudget(limits, checkpoint)
        val result = write(value, budget, "")
        budget.validation.validate(target, result, "")
        Json.stringifyChecked(result, limits.json, checkpoint)
        checkpoint()
        return result
    }
    internal fun encodeChecked(value: T, limits: CodecLimits, checkpoint: () -> Unit): String {
        val budget = ModelBudget(limits, checkpoint)
        val result = write(value, budget, "")
        budget.validation.validate(target, result, "")
        return Json.stringifyChecked(result, limits.json, checkpoint)
    }
}

/** Lower-level validation of selected canonical roots, including instruction
 * shapes that need a separate native-model representation gate.
 */
public object SchemaValidation {
    /** All roots in the checked generated program, retaining original identities. */
    public fun sources(): List<SourceLocation> = ValidationProgram.roots.keys.toList()

    /** Validate a JSON tree against an exact selected root.
     * @throws ValidationException On completed source invalidity.
     * @throws EvaluationException On an unknown root or incomplete evaluation.
     */
    public fun validate(source: SourceLocation, value: JsonValue, limits: CodecLimits = CodecLimits()) {
        val target = ValidationProgram.roots[source]
            ?: throw EvaluationException(ValidationFinding(source, "", "root was not selected"))
        val budget = ModelBudget(limits) {}
        val snapshot = budget.json(value, source, "")
        Json.stringify(snapshot, limits.json)
        budget.validation.validate(target, snapshot, "")
    }
}

internal fun childPath(path: String, key: String): String = path + "/" + key.replace("~", "~0").replace("/", "~1")

internal class ModelBudget(private val limits: CodecLimits, val checkpoint: () -> Unit) {
    val validation = ValidationSession(limits, checkpoint)
    private var depth = 0
    private var remaining = limits.maxModelSteps
    private var bytes = limits.maxModelBytes
    fun <T> at(source: SourceLocation, path: String, block: () -> T): T {
        checkpoint()
        if (remaining-- <= 0 || depth >= limits.json.maxDepth) fail(source, path)
        depth++
        try { return block() } finally { depth-- }
    }
    fun text(value: String, source: SourceLocation, path: String): String {
        val count = try { Json.utf8Size(value, bytes, checkpoint) }
        catch (error: JsonException) { if (error.kind == JsonErrorKind.RESOURCE_LIMIT) fail(source, path) else throw error }
        bytes -= count
        return value
    }
    fun collection(size: Int, source: SourceLocation, path: String) {
        checkpoint()
        // Kotlin map/associate preallocate from Collection.size. Admit the size
        // before invoking them, including lazy caller-owned collections.
        if (size < 0 || size > remaining || size > limits.json.maxValues) fail(source, path)
    }
    fun bytes(size: Int, source: SourceLocation, path: String) {
        checkpoint()
        if (size < 0 || size > bytes) fail(source, path)
        bytes -= size
    }
    fun representation(value: JsonValue): JsonValue {
        Json.stringifyChecked(value, limits.json, checkpoint)
        return value
    }
    fun json(value: JsonValue, source: SourceLocation, path: String): JsonValue = at(source, path) {
        when (value) {
            JsonNull, is JsonBoolean -> value
            is JsonNumber -> { text(value.token, source, path); value }
            is JsonString -> JsonString(text(value.value, source, path))
            is JsonArray -> {
                collection(value.values.size, source, path)
                JsonArray(value.values.mapIndexed { i, item -> json(item, source, childPath(path, i.toString())) })
            }
            is JsonObject -> {
                collection(value.values.size, source, path)
                JsonObject(value.values.entries.associate { (key, member) ->
                    text(key, source, path) to json(member, source, childPath(path, key))
                })
            }
        }
    }
    private fun fail(source: SourceLocation, path: String): Nothing =
        throw EvaluationException(ValidationFinding(source, path, "native traversal byte/visit/depth limit exceeded"))
}

internal object ValidationProgram {
    val nodes: List<JsonObject>
    val roots: Map<SourceLocation, Int>
    val limits: JsonObject
    init {
        val bytes = ValidationProgram::class.java.getResourceAsStream("validation.json")?.use { it.readNBytes(Json.MAX_PROGRAM_BYTES + 1) }
            ?: error("generated validation program is missing")
        val program = Json.parseProgram(bytes) as JsonObject
        check(program.text("version") == "suspect.validation.experimental.v1") { "unknown validation program version" }
        check(program.text("profile") == "oas31-jsonschema202012-static-subset") { "unknown validation profile" }
        nodes = program.array("nodes").map { it as JsonObject }
        check(nodes.size <= 16_384) { "program node limit exceeded" }
        roots = program.array("roots").associate { entry -> (entry as JsonObject).source() to entry.number("target") }
        limits = program.obj("limits")
        val supported = setOf("always", "type", "ref", "properties", "additionalProperties", "required", "items", "prefixItems", "allOf", "anyOf", "oneOf", "not", "bound", "multipleOf", "count", "enum", "const", "uniqueItems", "pattern")
        for (node in nodes) for (instruction in node.array("checks")) {
            val item = instruction as JsonObject
            check(item.text("op") in supported) { "unsupported validation instruction" }
            if (item.text("op") == "pattern") check(item.obj("program").text("version") == "suspect.pattern.experimental.v1") { "unknown pattern program version" }
        }
    }
}

internal fun JsonObject.text(key: String): String = (values.getValue(key) as JsonString).value
internal fun JsonObject.flag(key: String): Boolean = (values.getValue(key) as JsonBoolean).value
internal fun JsonObject.number(key: String): Int = (values.getValue(key) as JsonNumber).token.toInt()
internal fun JsonObject.obj(key: String): JsonObject = values.getValue(key) as JsonObject
internal fun JsonObject.array(key: String): List<JsonValue> = (values.getValue(key) as JsonArray).values
internal fun JsonObject.source(): SourceLocation = obj("source").let { SourceLocation(it.text("document"), it.text("pointer")) }

internal class ValidationSession(limits: CodecLimits, private val checkpoint: () -> Unit) {
    private val programLimits = ValidationProgram.limits
    private var steps = minOf(programLimits.number("maxEvaluationSteps"), limits.maxEvaluationSteps)
    private var equalitySteps = minOf(programLimits.number("maxEqualitySteps"), limits.maxEqualitySteps)
    private var numericSteps = limits.maxNumericSteps
    private var textBytes = limits.maxValidationBytes
    private val maxDepth = minOf(programLimits.number("maxDepth"), limits.json.maxDepth)
    private val maxErrors = programLimits.number("maxErrors")
    private val maxNumber = minOf(programLimits.number("maxNumberBytes"), limits.json.maxNumberBytes)
    private val active = mutableSetOf<Pair<Int, String>>()
    private val findings = mutableListOf<ValidationFinding>()

    fun validate(target: Int, value: JsonValue, path: String) {
        if (!eval(target, value, path, 0)) throw ValidationException(findings.toList())
    }
    fun matches(target: Int, value: JsonValue, path: String): Boolean = trial(target, value, path, 0)
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
        catch (error: JsonException) {
            if (error.kind == JsonErrorKind.RESOURCE_LIMIT) fail(source, path, "assertion text work limit exceeded") else throw error
        }
        textBytes -= amount
    }
    private fun reject(source: SourceLocation, path: String, message: String): Boolean {
        if (maxErrors == 0 || findings.size < maxErrors) findings.add(ValidationFinding(source, path, message))
        return false
    }
    private fun number(value: JsonNumber, source: SourceLocation, path: String): ExactDecimal {
        if (value.token.length > maxNumber) fail(source, path, "exact number byte limit exceeded")
        numeric(source, path, 1 + value.token.length / 16)
        return value.exact
    }
    private fun operand(check: JsonObject, path: String): ExactDecimal = number(JsonNumber.parse(check.text("value")), check.source(), path)
    private fun trial(target: Int, value: JsonValue, path: String, depth: Int): Boolean {
        val start = findings.size
        try { return eval(target, value, path, depth) }
        finally { while (findings.size > start) findings.removeAt(findings.lastIndex) }
    }
    private fun eval(target: Int, value: JsonValue, path: String, depth: Int): Boolean {
        val node = ValidationProgram.nodes.getOrNull(target)
            ?: fail(SourceLocation("", ""), path, "invalid generated schema target")
        val source = node.source()
        step(source, path)
        if (depth >= maxDepth) fail(source, path, "schema evaluation depth limit exceeded")
        val identity = target to path
        if (!active.add(identity)) fail(source, path, "recursive schema made no instance progress")
        try {
            var valid = true
            for (instruction in node.array("checks")) {
                val check = instruction as JsonObject
                val at = check.source()
                step(at, path)
                val okay = when (check.text("op")) {
                    "always" -> check.flag("value") || reject(at, path, "false schema rejects every value")
                    "type" -> {
                        val accepted = check.array("types").map { (it as JsonString).value }
                        val match = when (value) {
                            JsonNull -> "null" in accepted
                            is JsonBoolean -> "boolean" in accepted
                            is JsonNumber -> "number" in accepted || ("integer" in accepted && number(value, at, path).integral)
                            is JsonString -> "string" in accepted
                            is JsonArray -> "array" in accepted
                            is JsonObject -> "object" in accepted
                        }
                        match || reject(at, path, "instance does not match the declared type")
                    }
                    "ref" -> eval(check.number("target"), value, path, depth + 1)
                    "properties" -> {
                        var okay = true
                        if (value is JsonObject) for (property in check.array("properties")) {
                            step(at, path)
                            val p = property as JsonObject
                            val key = p.text("name")
                            value.values[key]?.let { if (!eval(p.number("target"), it, childPath(path, key), depth + 1)) okay = false }
                        }
                        okay
                    }
                    "additionalProperties" -> {
                        var okay = true
                        val declared = check.array("declared").map { (it as JsonString).value }.toSet()
                        if (value is JsonObject) for ((key, member) in value.values) {
                            step(at, path)
                            if (key !in declared && !eval(check.number("target"), member, childPath(path, key), depth + 1)) okay = false
                        }
                        okay
                    }
                    "required" -> {
                        var okay = true
                        if (value is JsonObject) for (name in check.array("names")) {
                            step(at, path)
                            if ((name as JsonString).value !in value.values) {
                                reject(at, childPath(path, name.value), "required property is absent"); okay = false
                            }
                        }
                        okay
                    }
                    "items", "prefixItems" -> {
                        var okay = true
                        if (value is JsonArray) {
                            val targets = if (check.text("op") == "prefixItems") check.array("targets") else null
                            val start = if (targets == null) check.number("start") else 0
                            val end = if (targets == null) value.values.size else minOf(value.values.size, targets.size)
                            for (i in start until end) {
                                step(at, path)
                                val next = targets?.get(i)?.let { (it as JsonNumber).token.toInt() } ?: check.number("target")
                                if (!eval(next, value.values[i], childPath(path, i.toString()), depth + 1)) okay = false
                            }
                        }
                        okay
                    }
                    "allOf", "anyOf", "oneOf" -> {
                        var matches = 0
                        val operation = check.text("op")
                        val targets = check.array("targets")
                        for (branch in targets) {
                            step(at, path)
                            val next = (branch as JsonNumber).token.toInt()
                            if (if (operation == "allOf") eval(next, value, path, depth + 1) else trial(next, value, path, depth + 1)) matches++
                        }
                        (when (operation) { "allOf" -> matches == targets.size; "anyOf" -> matches > 0; else -> matches == 1 }) || reject(at, path, "composition matched $matches alternatives")
                    }
                    "not" -> !trial(check.number("target"), value, path, depth + 1) || reject(at, path, "instance matches the negated schema")
                    "bound" -> {
                        if (value !is JsonNumber) true else {
                            val compare = number(value, at, path).compareTo(operand(check, path))
                            val okay = if (check.flag("maximum")) compare < 0 || (compare == 0 && !check.flag("exclusive")) else compare > 0 || (compare == 0 && !check.flag("exclusive"))
                            okay || reject(at, path, "number violates its exact bound")
                        }
                    }
                    "multipleOf" -> value !is JsonNumber || number(value, at, path).multipleOf(operand(check, path)) { numeric(at, path, it) } || reject(at, path, "number is not an exact multiple")
                    "count" -> {
                        val count = when (check.text("target")) {
                            "string" -> (value as? JsonString)?.value?.let { text(at, path, it); it.codePointCount(0, it.length) }
                            "array" -> (value as? JsonArray)?.values?.size
                            "object" -> (value as? JsonObject)?.values?.size
                            else -> fail(at, path, "unknown cardinality target")
                        }
                        if (count == null) true else {
                            val compare = number(JsonNumber.of(count.toLong()), at, path).compareTo(operand(check, path))
                            (if (check.flag("maximum")) compare <= 0 else compare >= 0) || reject(at, path, "instance violates its cardinality bound")
                        }
                    }
                    "const" -> equal(value, check.values.getValue("value"), at, path, 0) || reject(at, path, "instance differs from const")
                    "enum" -> {
                        var found = false
                        for (candidate in check.array("values")) {
                            step(at, path)
                            if (equal(value, candidate, at, path, 0)) found = true
                        }
                        found || reject(at, path, "instance is not an enum member")
                    }
                    "uniqueItems" -> {
                        var unique = true
                        if (value is JsonArray) for (i in value.values.indices) for (j in 0 until i) {
                            step(at, path)
                            if (equal(value.values[i], value.values[j], at, path, 0)) unique = false
                        }
                        unique || reject(at, path, "array items are not unique")
                    }
                    "pattern" -> value !is JsonString || pattern(check.obj("program"), value.value, at, path) || reject(at, path, "string does not match the portable pattern")
                    else -> fail(at, path, "unsupported compiled validation instruction")
                }
                if (!okay) valid = false
            }
            return valid
        } finally { active.remove(identity) }
    }
    private fun equal(a: JsonValue, b: JsonValue, source: SourceLocation, path: String, depth: Int): Boolean {
        checkpoint()
        if (equalitySteps-- <= 0 || depth >= maxDepth) fail(source, path, "structural equality limit exceeded")
        return when {
            a is JsonNumber && b is JsonNumber -> number(a, source, path).compareTo(number(b, source, path)) == 0
            a is JsonString && b is JsonString -> { text(source, path, a.value); text(source, path, b.value); a.value == b.value }
            a is JsonArray && b is JsonArray -> {
                if (a.values.size != b.values.size) false else {
                    var equal = true
                    for (i in a.values.indices) if (!equal(a.values[i], b.values[i], source, path, depth + 1)) equal = false
                    equal
                }
            }
            a is JsonObject && b is JsonObject -> {
                if (a.values.size != b.values.size) false else {
                    var equal = true
                    for ((key, value) in a.values) {
                        text(source, path, key)
                        val other = b.values[key]
                        if (other == null || !equal(value, other, source, path, depth + 1)) equal = false
                    }
                    equal
                }
            }
            else -> a == b
        }
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
                    if (scalar < (bounds[0] as JsonNumber).token.toInt()) break
                    if (scalar <= (bounds[1] as JsonNumber).token.toInt()) { next.add(state.number("target")); break }
                }
            }
            seeds = next
        }
    }
}
