package __PACKAGE__

import java.math.BigInteger
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/** Exact JSON values. Collections may be caller-owned; codecs snapshot and revalidate them. */
public sealed interface JsonValue

/** JSON null. Nullable native model fields instead use Kotlin `null`. */
public data object JsonNull : JsonValue

/** A decoded Unicode string; JSON I/O rejects unpaired surrogates.
 * @property value Decoded string, without Unicode normalization.
 */
public data class JsonString(/** Decoded Unicode string. */ public val value: String) : JsonValue

/** A JSON boolean, distinct from every number.
 * @property value Boolean value.
 */
public data class JsonBoolean(/** Boolean value. */ public val value: Boolean) : JsonValue

/** A JSON array, retaining every element.
 * @property values Elements in wire order. Do not mutate during a codec call.
 */
public data class JsonArray(/** Elements in wire order. */ public val values: List<JsonValue>) : JsonValue

/** A JSON object, retaining every decoded key.
 * @property values Members. Do not mutate during a codec call.
 */
public data class JsonObject(/** Exact decoded member names and values. */ public val values: Map<String, JsonValue>) : JsonValue

/** An exact JSON number with a symbolic, unbounded decimal exponent.
 *
 * [parse] preserves the original token. Equality, hashing, ordering and integer
 * conversion are mathematical: `1`, `1.0` and `1e0` compare equal. No binary
 * floating-point or BigDecimal scale conversion is performed.
 * @property token Original validated ASCII numeric token.
 */
public class JsonNumber private constructor(/** Original validated ASCII numeric token. */ public val token: String) : JsonValue, Comparable<JsonNumber> {
    internal val exact: ExactDecimal by lazy(LazyThreadSafetyMode.PUBLICATION) { ExactDecimal.parse(token) }

    /** Original spelling, suitable for exact JSON emission. */
    override fun toString(): String = token
    /** Exact mathematical equality, independent of spelling. */
    override fun equals(other: Any?): Boolean = other is JsonNumber && exact == other.exact
    /** Mathematical hash, including one hash for every spelling of zero. */
    override fun hashCode(): Int = exact.hashCode()
    /** Exact order without exponent-sized allocation. */
    override fun compareTo(other: JsonNumber): Int = exact.compareTo(other.exact)
    /** Whether this value has no fractional component. */
    public fun isInteger(): Boolean = exact.integral

    /** Convert a mathematical integer, expanding at most 4096 decimal digits.
     * @throws ArithmeticException If this number has a fractional component.
     * @throws JsonException If exponent expansion would exceed the finite limit.
     */
    public fun toBigIntegerExact(): BigInteger {
        val number = exact
        if (!number.integral) throw ArithmeticException("JSON number is not an integer")
        if (number.sign == 0) return BigInteger.ZERO
        if (number.exponent + BigInteger.valueOf(number.digits.length.toLong()) > BigInteger.valueOf(4096)) {
            throw JsonException("integer expansion exceeds 4096 digits", 0, JsonErrorKind.RESOURCE_LIMIT)
        }
        val magnitude = BigInteger(number.digits) * BigInteger.TEN.pow(number.exponent.toInt())
        return if (number.sign < 0) magnitude.negate() else magnitude
    }

    /** Convert exactly to Long, rejecting fractions, overflow and huge expansion. */
    public fun toLongExact(): Long = toBigIntegerExact().longValueExact()

    /** Exact-number factories. */
    public companion object {
        private val syntax = Regex("-?(?:0|[1-9][0-9]*)(?:\\.[0-9]+)?(?:[eE][+-]?[0-9]+)?")

        /** Parse a strict JSON number of at most 4096 ASCII bytes. */
        public fun parse(token: String): JsonNumber {
            if (token.length > 4096) throw JsonException("number exceeds 4096 bytes", 0, JsonErrorKind.RESOURCE_LIMIT)
            if (!syntax.matches(token)) throw JsonException("invalid JSON number", 0)
            return JsonNumber(token)
        }

        /** Represent a Kotlin integer exactly. */
        public fun of(value: Long): JsonNumber = JsonNumber(value.toString())
        /** Represent a BigInteger within the numeric-token limit exactly. */
        public fun of(value: BigInteger): JsonNumber = parse(value.toString())
    }
}

/** Stable JSON failure categories. */
public enum class JsonErrorKind {
    /** Malformed JSON syntax, Unicode or UTF-8. */ SYNTAX,
    /** A finite byte, number, depth, value or work limit was exhausted. */ RESOURCE_LIMIT,
}

/** A bounded JSON error. Messages never include input tokens or object keys.
 * @property offset UTF-16 position in text; zero for byte/size errors.
 * @property kind Syntax versus incomplete resource-limited work.
 */
public class JsonException(message: String, /** UTF-16 text offset, or zero for byte errors. */ public val offset: Int, /** Syntax versus resource exhaustion. */ public val kind: JsonErrorKind = JsonErrorKind.SYNTAX) :
    IllegalArgumentException("$message at offset $offset")

/** Finite JSON policy; callers may lower each ceiling, including to zero.
 * @property maxBytes Maximum input/output UTF-8 bytes, at most 4 MiB.
 * @property maxDepth Maximum nesting depth, at most 128.
 * @property maxValues Maximum visited values, at most 100000.
 * @property maxNumberBytes Maximum bytes in one numeric token, at most 4096.
 * @property maxWorkBytes Maximum scanned/copied text units, at most 16 MiB.
 */
public data class JsonLimits(
    /** Input/output UTF-8 byte ceiling. */
    public val maxBytes: Int = 4 * 1024 * 1024,
    /** Maximum nesting depth. */
    public val maxDepth: Int = 128,
    /** Maximum visited values. */
    public val maxValues: Int = 100_000,
    /** Maximum bytes in one exact numeric token. */
    public val maxNumberBytes: Int = 4096,
    /** Maximum scanned/copied text units. */
    public val maxWorkBytes: Int = 16 * 1024 * 1024,
) {
    init {
        require(maxBytes in 0..4194304 && maxDepth in 0..128 && maxValues in 0..100000
            && maxNumberBytes in 0..4096 && maxWorkBytes in 0..16777216) { "JSON limits must be within the finite profile" }
    }
}

internal class JsonBudget(
    val limits: JsonLimits,
    val checkpoint: () -> Unit,
    private var remaining: Int = limits.maxValues,
    private var work: Int = limits.maxWorkBytes,
) {
    fun value(depth: Int) {
        checkpoint()
        if (depth >= limits.maxDepth || remaining-- <= 0) resource("JSON nesting/value limit exceeded")
    }
    fun spend(amount: Int) {
        if (amount > work) resource("JSON work limit exceeded")
        work -= amount
        checkpoint()
    }
    fun resource(message: String): Nothing = throw JsonException(message, 0, JsonErrorKind.RESOURCE_LIMIT)
}

/** Strict, finite exact JSON I/O. Duplicate keys and trailing data are rejected. */
public object Json {
    internal const val MAX_BYTES: Int = 4 * 1024 * 1024
    internal const val MAX_PROGRAM_BYTES: Int = 16 * 1024 * 1024

    // Generated metadata has its own finite load budget, separate from API payloads.
    internal fun parseProgram(bytes: ByteArray): JsonValue {
        if (bytes.size > MAX_PROGRAM_BYTES) throw JsonException("program byte limit exceeded", 0, JsonErrorKind.RESOURCE_LIMIT)
        return Parser(decodeUtf8(bytes), JsonBudget(JsonLimits(), {}, 1_000_000, 4 * MAX_PROGRAM_BYTES)).document()
    }

    /** Parse a document, retaining exact numbers and distinct Unicode keys. */
    public fun parse(text: String, limits: JsonLimits = JsonLimits()): JsonValue = parseChecked(text, limits) {}
    /** Decode UTF-8 strictly, then parse a document. */
    public fun parse(bytes: ByteArray, limits: JsonLimits = JsonLimits()): JsonValue = parseChecked(bytes, limits) {}
    /** Encode deterministically, checking every reachable value and output byte. */
    public fun stringify(value: JsonValue, limits: JsonLimits = JsonLimits()): String = stringifyChecked(value, limits) {}

    internal fun parseChecked(text: String, limits: JsonLimits, checkpoint: () -> Unit): JsonValue {
        utf8Size(text, limits.maxBytes, checkpoint)
        return Parser(text, JsonBudget(limits, checkpoint)).document()
    }
    internal fun parseChecked(bytes: ByteArray, limits: JsonLimits, checkpoint: () -> Unit): JsonValue {
        checkpoint()
        if (bytes.size > limits.maxBytes) throw JsonException("JSON byte limit exceeded", 0, JsonErrorKind.RESOURCE_LIMIT)
        val text = decodeUtf8(bytes)
        checkpoint()
        return Parser(text, JsonBudget(limits, checkpoint)).document()
    }

    private fun decodeUtf8(bytes: ByteArray): String = try {
        Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
    } catch (_: java.nio.charset.CharacterCodingException) { throw JsonException("invalid UTF-8", 0) }
    internal fun stringifyChecked(value: JsonValue, limits: JsonLimits, checkpoint: () -> Unit): String {
        val writer = Writer(JsonBudget(limits, checkpoint))
        writer.value(value, 0)
        return writer.out.toString()
    }

    private class Parser(private val text: String, private val budget: JsonBudget) {
        private var at = 0
        private fun fail(message: String): Nothing = throw JsonException(message, at)
        private fun space() {
            val start = at
            while (at < text.length && text[at] in " \t\r\n") {
                at++
                if ((at - start) % 1024 == 0) budget.checkpoint()
            }
            budget.spend(at - start)
        }
        fun document(): JsonValue {
            val result = value(0)
            space()
            if (at != text.length) fail("trailing JSON data")
            return result
        }
        private fun value(depth: Int): JsonValue {
            budget.value(depth)
            space()
            if (at == text.length) fail("unexpected end of JSON")
            return when (text[at]) {
                'n' -> { literal("null"); JsonNull }
                't' -> { literal("true"); JsonBoolean(true) }
                'f' -> { literal("false"); JsonBoolean(false) }
                '"' -> JsonString(string())
                '[' -> {
                    at++; space()
                    val values = mutableListOf<JsonValue>()
                    if (!take(']')) {
                        do { values.add(value(depth + 1)); space() } while (take(','))
                        expect(']')
                    }
                    JsonArray(values.toList())
                }
                '{' -> {
                    at++; space()
                    val values = linkedMapOf<String, JsonValue>()
                    if (!take('}')) {
                        do {
                            space()
                            val key = string()
                            if (values.containsKey(key)) fail("duplicate JSON object key")
                            space(); expect(':')
                            values[key] = value(depth + 1)
                            space()
                        } while (take(','))
                        expect('}')
                    }
                    JsonObject(values.toMap())
                }
                '-', in '0'..'9' -> {
                    val start = at
                    while (at < text.length && text[at] in "0123456789eE+.-") {
                        if (at - start >= budget.limits.maxNumberBytes) budget.resource("number byte limit exceeded")
                        at++
                    }
                    budget.spend(at - start)
                    JsonNumber.parse(text.substring(start, at))
                }
                else -> fail("invalid JSON value")
            }
        }
        private fun literal(expected: String) {
            budget.spend(expected.length)
            if (!text.startsWith(expected, at)) fail("invalid JSON literal")
            at += expected.length
        }
        private fun take(c: Char): Boolean {
            if (at < text.length && text[at] == c) { at++; budget.spend(1); return true }
            return false
        }
        private fun expect(c: Char) { if (!take(c)) fail("unexpected JSON delimiter") }
        private fun string(): String {
            expect('"')
            val result = StringBuilder()
            while (at < text.length) {
                budget.spend(1)
                val c = text[at++]
                when {
                    c == '"' -> return result.toString().also { utf8Size(it, budget.limits.maxBytes, budget.checkpoint) }
                    c < ' ' -> fail("control character in JSON string")
                    c == '\\' -> {
                        if (at == text.length) fail("unterminated escape")
                        budget.spend(1)
                        result.append(when (text[at++]) {
                            '"' -> '"'; '\\' -> '\\'; '/' -> '/'; 'b' -> '\b'; 'f' -> '\u000C'
                            'n' -> '\n'; 'r' -> '\r'; 't' -> '\t'
                            'u' -> {
                                if (text.length - at < 4) fail("short Unicode escape")
                                budget.spend(4)
                                var code = 0
                                repeat(4) {
                                    val digit = text[at++]
                                    if (digit !in "0123456789abcdefABCDEF") fail("invalid Unicode escape")
                                    code = code * 16 + digit.digitToInt(16)
                                }
                                code.toChar()
                            }
                            else -> fail("invalid JSON escape")
                        })
                    }
                    else -> result.append(c)
                }
            }
            fail("unterminated JSON string")
        }
    }

    private class Writer(private val budget: JsonBudget) {
        val out = StringBuilder()
        private var bytes = 0
        private fun append(text: String, size: Int = text.length) {
            if (size > budget.limits.maxBytes - bytes) budget.resource("JSON byte limit exceeded")
            bytes += size
            out.append(text)
        }
        fun value(value: JsonValue, depth: Int) {
            budget.value(depth)
            when (value) {
                JsonNull -> append("null")
                is JsonBoolean -> append(value.value.toString())
                is JsonNumber -> {
                    if (value.token.length > budget.limits.maxNumberBytes) budget.resource("number byte limit exceeded")
                    budget.spend(value.token.length); append(value.token)
                }
                is JsonString -> string(value.value)
                is JsonArray -> {
                    if (value.values.size > budget.limits.maxValues) budget.resource("JSON collection limit exceeded")
                    append("[")
                    value.values.forEachIndexed { index, item -> if (index > 0) append(","); value(item, depth + 1) }
                    append("]")
                }
                is JsonObject -> {
                    if (value.values.size > budget.limits.maxValues) budget.resource("JSON collection limit exceeded")
                    budget.spend(value.values.size)
                    append("{")
                    value.values.keys.sorted().forEachIndexed { index, key ->
                        if (index > 0) append(",")
                        string(key); append(":"); value(value.values.getValue(key), depth + 1)
                    }
                    append("}")
                }
            }
        }
        private fun string(value: String) {
            budget.spend(value.length)
            utf8Size(value, budget.limits.maxBytes, budget.checkpoint)
            append("\"")
            var index = 0
            while (index < value.length) {
                if (index % 1024 == 0) budget.checkpoint()
                val c = value[index++]
                when (c) {
                    '"' -> append("\\\""); '\\' -> append("\\\\")
                    '\n' -> append("\\n"); '\r' -> append("\\r"); '\t' -> append("\\t")
                    else -> when {
                        c < ' ' -> append("\\u" + c.code.toString(16).padStart(4, '0'))
                        c.isHighSurrogate() -> append("$c${value[index++]}", 4)
                        else -> append(c.toString(), if (c.code < 128) 1 else if (c.code < 2048) 2 else 3)
                    }
                }
            }
            append("\"")
        }
    }

    internal fun unicode(text: String) { utf8Size(text, Int.MAX_VALUE) {} }
    internal fun utf8Size(text: String, maximum: Int, checkpoint: () -> Unit): Int {
        if (text.length > maximum) throw JsonException("JSON byte limit exceeded", 0, JsonErrorKind.RESOURCE_LIMIT)
        var index = 0
        var bytes = 0
        while (index < text.length) {
            if (index % 1024 == 0) checkpoint()
            val c = text[index++]
            bytes += when {
                c.isHighSurrogate() -> {
                    if (index == text.length || !text[index++].isLowSurrogate()) throw JsonException("unpaired UTF-16 surrogate", index - 1)
                    4
                }
                c.isLowSurrogate() -> throw JsonException("unpaired UTF-16 surrogate", index - 1)
                c.code < 128 -> 1
                c.code < 2048 -> 2
                else -> 3
            }
            if (bytes > maximum) throw JsonException("JSON byte limit exceeded", 0, JsonErrorKind.RESOURCE_LIMIT)
        }
        checkpoint()
        return bytes
    }
}

// Normalization is proportional to token length, never to exponent magnitude.
internal data class ExactDecimal(val sign: Int, val digits: String, val exponent: BigInteger) : Comparable<ExactDecimal> {
    val integral: Boolean get() = sign == 0 || exponent.signum() >= 0
    override fun compareTo(other: ExactDecimal): Int {
        if (sign != other.sign) return sign.compareTo(other.sign)
        if (sign == 0) return 0
        val order = (exponent + BigInteger.valueOf(digits.length.toLong()))
            .compareTo(other.exponent + BigInteger.valueOf(other.digits.length.toLong()))
        if (order != 0) return sign * order
        for (i in 0 until maxOf(digits.length, other.digits.length)) {
            val difference = digits.getOrElse(i) { '0' }.compareTo(other.digits.getOrElse(i) { '0' })
            if (difference != 0) return sign * difference
        }
        return 0
    }
    fun multipleOf(other: ExactDecimal, spend: (Int) -> Unit): Boolean {
        check(other.sign > 0) { "compiled divisor must be positive" }
        if (sign == 0) return true
        val shift = exponent - other.exponent
        if (shift.signum() < 0) return false
        spend(1 + digits.length * other.digits.length / 64)
        val numerator = BigInteger(digits)
        val divisor = BigInteger(other.digits)
        var denominator = divisor / numerator.gcd(divisor)
        var twos = 0
        var fives = 0
        while (denominator.and(BigInteger.ONE) == BigInteger.ZERO) {
            spend(1 + denominator.bitLength() / 64)
            denominator = denominator.shiftRight(1); twos++
        }
        val five = BigInteger.valueOf(5)
        while (true) {
            spend(1 + denominator.bitLength() / 64)
            if (denominator.mod(five) != BigInteger.ZERO) break
            denominator /= five; fives++
        }
        return denominator == BigInteger.ONE && shift >= BigInteger.valueOf(maxOf(twos, fives).toLong())
    }
    companion object {
        fun parse(token: String): ExactDecimal {
            val negative = token.startsWith('-')
            val unsigned = if (negative) token.substring(1) else token
            val exponentAt = unsigned.indexOfAny(charArrayOf('e', 'E'))
            val mantissa = if (exponentAt < 0) unsigned else unsigned.substring(0, exponentAt)
            var exponent = if (exponentAt < 0) BigInteger.ZERO else BigInteger(unsigned.substring(exponentAt + 1))
            val dot = mantissa.indexOf('.')
            if (dot >= 0) exponent -= BigInteger.valueOf((mantissa.length - dot - 1).toLong())
            val digits = mantissa.replace(".", "").trimStart('0')
            if (digits.isEmpty()) return ExactDecimal(0, "0", BigInteger.ZERO)
            val trimmed = digits.trimEnd('0')
            exponent += BigInteger.valueOf((digits.length - trimmed.length).toLong())
            return ExactDecimal(if (negative) -1 else 1, trimmed, exponent)
        }
    }
}
