package {package};

import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Immutable exact JSON, strict Unicode/UTF-8 and finite parser/writer work. */
public final class JsonRuntime {
    private JsonRuntime() {}
    /** Hard input/output allocation ceiling. */
    public static final int MAX_BYTES = 8 * 1024 * 1024;
    /** Hard native recursion ceiling, independent of the schema graph. */
    public static final int MAX_DEPTH = 128;
    /** Hard numeric-token ceiling; exponents never expand implicitly. */
    public static final int MAX_NUMBER_BYTES = 65536;

    /** Per-call limits. Zero permits no bytes/work; depth zero permits scalars.
     * @param maxInputBytes input UTF-8 bytes
     * @param maxOutputBytes output UTF-8 bytes
     * @param maxDepth container nesting
     * @param maxNumberBytes bytes in one exact numeric token
     * @param maxWork total parser/writer visits and processed bytes
     */
    public record Limits(int maxInputBytes, int maxOutputBytes, int maxDepth, int maxNumberBytes, long maxWork) {
        /** Check this finite resource policy. */
        public Limits {
            if (maxInputBytes < 0 || maxInputBytes > MAX_BYTES || maxOutputBytes < 0 || maxOutputBytes > MAX_BYTES
                    || maxDepth < 0 || maxDepth > MAX_DEPTH || maxNumberBytes < 0 || maxNumberBytes > MAX_NUMBER_BYTES
                    || maxWork < 0 || maxWork > 256L * MAX_BYTES)
                throw new IllegalArgumentException("invalid JSON resource limits");
        }
        /** Default bounded JSON policy. @return immutable limits */
        public static Limits defaults() { return new Limits(MAX_BYTES, MAX_BYTES, MAX_DEPTH, MAX_NUMBER_BYTES, 32L * 1024 * 1024); }
        /** Change the input ceiling. @param value bytes @return new policy */
        public Limits withInputBytes(int value) { return new Limits(value, maxOutputBytes, maxDepth, maxNumberBytes, maxWork); }
        /** Change the output ceiling. @param value bytes @return new policy */
        public Limits withOutputBytes(int value) { return new Limits(maxInputBytes, value, maxDepth, maxNumberBytes, maxWork); }
        /** Change the work ceiling. @param value work units @return new policy */
        public Limits withWork(long value) { return new Limits(maxInputBytes, maxOutputBytes, maxDepth, maxNumberBytes, value); }
        /** Change the nesting ceiling. @param value depth @return new policy */
        public Limits withDepth(int value) { return new Limits(maxInputBytes, maxOutputBytes, value, maxNumberBytes, maxWork); }
    }

    /** A JSON value. Java null is deliberately outside this sealed domain. */
    public sealed interface JsonValue permits JsonNull, JsonBoolean, JsonString, JsonNumber, JsonArray, JsonObject {}
    /** The JSON null value. */
    public enum JsonNull implements JsonValue {
        /** The sole JSON null value. */
        INSTANCE
    }
    /** A JSON boolean. @param value its value */
    public record JsonBoolean(boolean value) implements JsonValue {}
    /** A Unicode scalar string. @param value well-formed UTF-16 */
    public record JsonString(String value) implements JsonValue {
        /** Check Unicode without normalizing it. */
        public JsonString { unicode(value); }
    }
    /** An immutable array. @param values JSON values, never Java null */
    public record JsonArray(List<JsonValue> values) implements JsonValue {
        /** Snapshot the container; its sealed descendants are immutable. */
        public JsonArray {
            if (values.size() > MAX_BYTES) throw resource("JSON array allocation ceiling");
            values = List.copyOf(values);
        }
    }
    /** An immutable object with decoded, exact member names.
     * @param values well-formed Unicode keys and non-null JSON values
     */
    public record JsonObject(Map<String, JsonValue> values) implements JsonValue {
        /** Snapshot the container; its sealed descendants are immutable. */
        public JsonObject {
            if (values.size() > MAX_BYTES) throw resource("JSON object allocation ceiling");
            var copy = new LinkedHashMap<String, JsonValue>();
            values.forEach((key, value) -> { unicode(key); copy.put(key, Objects.requireNonNull(value)); });
            values = Collections.unmodifiableMap(copy);
        }
    }

    /** An opaque exact decimal token. Comparison and integrality use a symbolic
     * exponent, including arbitrarily padded exponent spellings. No double or
     * bounded decimal conversion participates in codecs or schema validation.
     */
    public static final class JsonNumber implements JsonValue, Comparable<JsonNumber> {
        private final String token;
        private final int sign;
        private final String digits;
        private final BigInteger exponent;
        private JsonNumber(String token, int sign, String digits, BigInteger exponent) {
            this.token = token; this.sign = sign; this.digits = digits; this.exponent = exponent;
        }
        /** Original exact spelling. @return JSON numeric token */
        public String token() { return token; }
        /** Normalized symbolic exponent. @return exponent without expansion */
        public BigInteger exponent() { return exponent; }
        /** Mathematical integrality, independent of wire spelling. @return true for integers */
        public boolean isInteger() { return sign == 0 || exponent.signum() >= 0; }
        /** Mathematical sign; all zero spellings have sign zero. @return -1, 0 or 1 */
        public int signum() { return sign; }
        /** Exact construction from a machine integer. @param value integer @return exact number */
        public static JsonNumber of(long value) { return parse(Long.toString(value)); }
        /** Parse one token with the default finite policy.
         * @param token exact JSON numeric token
         * @return immutable exact number
         * @throws JsonError for malformed syntax or exhausted numeric work
         */
        public static JsonNumber parse(String token) { return parse(token, new Budget(Limits.defaults())); }
        private static JsonNumber parse(String token, Budget budget) {
            Objects.requireNonNull(token);
            if (token.length() > budget.limits.maxNumberBytes()) throw resource("numeric token byte ceiling");
            budget.spend(token.length());
            if (numberEnd(token, 0) != token.length()) throw invalid("invalid numeric token", 0);
            boolean negative = token.startsWith("-");
            int start = negative ? 1 : 0;
            int e = Math.max(token.indexOf('e'), token.indexOf('E'));
            String mantissa = token.substring(start, e < 0 ? token.length() : e);
            BigInteger exponent = BigInteger.ZERO;
            if (e >= 0) {
                int p = e + 1;
                boolean exponentNegative = token.charAt(p) == '-';
                if (token.charAt(p) == '-' || token.charAt(p) == '+') p++;
                while (p < token.length() && token.charAt(p) == '0') p++;
                int significant = token.length() - p;
                // Charge before BigInteger's non-linear decimal conversion. Leading
                // padding is scanned, but is never mistaken for exponent magnitude.
                budget.spend((long) significant * significant / 16);
                if (significant > 0) {
                    exponent = new BigInteger(token.substring(p));
                    if (exponentNegative) exponent = exponent.negate();
                }
            }
            int dot = mantissa.indexOf('.');
            if (dot >= 0) exponent = exponent.subtract(BigInteger.valueOf(mantissa.length() - dot - 1));
            String digits = mantissa.replace(".", "");
            int first = 0, last = digits.length();
            while (first < last && digits.charAt(first) == '0') first++;
            if (first == last) return new JsonNumber(token, 0, "0", BigInteger.ZERO);
            while (digits.charAt(last - 1) == '0') last--;
            exponent = exponent.add(BigInteger.valueOf(digits.length() - last));
            return new JsonNumber(token, negative ? -1 : 1, digits.substring(first, last), exponent);
        }
        /** Explicit finite conversion. Fractions never truncate.
         * @return exact integer with at most 65536 decimal digits
         * @throws ArithmeticException if fractional
         * @throws JsonError if expansion exceeds the ceiling
         */
        public BigInteger exactIntegerValue() { return exactIntegerValue(MAX_NUMBER_BYTES); }
        /** Explicit finite conversion with a caller-supplied digit ceiling.
         * @param maxDigits positive maximum, at most 65536
         * @return exact integer
         * @throws ArithmeticException if fractional
         * @throws JsonError if expansion exceeds the ceiling
         */
        public BigInteger exactIntegerValue(int maxDigits) {
            if (maxDigits < 1 || maxDigits > MAX_NUMBER_BYTES) throw resource("integer expansion ceiling");
            if (!isInteger()) throw new ArithmeticException("JSON number is not an integer");
            if (sign == 0) return BigInteger.ZERO;
            if (exponent.add(BigInteger.valueOf(digits.length())).compareTo(BigInteger.valueOf(maxDigits)) > 0)
                throw resource("integer expansion ceiling");
            return new BigInteger(digits).multiply(BigInteger.TEN.pow(exponent.intValueExact())).multiply(BigInteger.valueOf(sign));
        }
        @Override public int compareTo(JsonNumber other) {
            Objects.requireNonNull(other);
            if (sign != other.sign) return Integer.compare(sign, other.sign);
            if (sign == 0) return 0;
            int order = exponent.add(BigInteger.valueOf(digits.length())).compareTo(other.exponent.add(BigInteger.valueOf(other.digits.length())));
            if (order == 0) {
                for (int i = 0; i < Math.max(digits.length(), other.digits.length()); i++) {
                    char a = i < digits.length() ? digits.charAt(i) : '0';
                    char b = i < other.digits.length() ? other.digits.charAt(i) : '0';
                    if (a != b) { order = Character.compare(a, b); break; }
                }
            }
            return sign * Integer.signum(order);
        }
        boolean multipleOf(JsonNumber divisor, java.util.function.LongConsumer spend) {
            if (divisor.sign <= 0) throw invalid("invalid checked divisor", -1);
            if (sign == 0) return true;
            BigInteger shift = exponent.subtract(divisor.exponent);
            if (shift.signum() < 0) return false;
            long size = (long) digits.length() + divisor.digits.length();
            spend.accept(size + size * size / 16);
            BigInteger denominator = new BigInteger(divisor.digits);
            for (int prime : new int[]{2, 5}) {
                BigInteger p = BigInteger.valueOf(prime);
                for (int powers = 0; BigInteger.valueOf(powers).compareTo(shift) < 0; powers++) {
                    spend.accept(1L + denominator.bitLength() / 32);
                    BigInteger[] divided = denominator.divideAndRemainder(p);
                    if (divided[1].signum() != 0) break;
                    denominator = divided[0];
                }
            }
            spend.accept(size + size * size / 16);
            return new BigInteger(digits).mod(denominator).signum() == 0;
        }
        @Override public boolean equals(Object value) { return value instanceof JsonNumber other && compareTo(other) == 0; }
        @Override public int hashCode() { return Objects.hash(sign, digits, exponent); }
        @Override public String toString() { return token; }
    }

    /** Structured JSON syntax, resource or cancellation failure. Messages never
     * embed an input token, key or string.
     */
    public static final class JsonError extends IllegalArgumentException {
        private static final long serialVersionUID = 1L;
        private final String kind;
        private final int offset;
        private JsonError(String kind, String message, int offset) {
            super("JSON " + kind + (offset < 0 ? "" : " at offset " + offset) + ": " + message);
            this.kind = kind; this.offset = offset;
        }
        /** Stable classification. @return invalid, resource, or cancelled */
        public String kind() { return kind; }
        /** UTF-16 input offset, or -1 if inapplicable. @return offset */
        public int offset() { return offset; }
    }
    private static JsonError invalid(String message, int offset) { return new JsonError("invalid", message, offset); }
    private static JsonError resource(String message) { return new JsonError("resource", message, -1); }

    static final class Budget {
        final Limits limits;
        private long work;
        Budget(Limits limits) { this.limits = Objects.requireNonNull(limits); work = limits.maxWork(); }
        void spend(long amount) {
            if (Thread.currentThread().isInterrupted()) throw new JsonError("cancelled", "interrupted", -1);
            if (amount < 0 || work < amount) throw resource("JSON work ceiling");
            work -= amount;
        }
    }

    /** Parse strict UTF-8. @param bytes document @return immutable JSON */
    public static JsonValue parse(byte[] bytes) { return parse(bytes, Limits.defaults()); }
    /** Parse with an explicit policy. @param bytes document @param limits finite policy @return immutable JSON */
    public static JsonValue parse(byte[] bytes, Limits limits) { return parse(bytes, new Budget(limits)); }
    static JsonValue parse(byte[] bytes, Budget budget) {
        Objects.requireNonNull(bytes);
        if (bytes.length > budget.limits.maxInputBytes()) throw resource("JSON input byte ceiling");
        budget.spend(bytes.length);
        try {
            String text = StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                    .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString();
            return parseText(text, budget);
        } catch (CharacterCodingException error) { throw invalid("invalid UTF-8", -1); }
    }
    /** Parse a complete Java string. @param text document @return immutable JSON */
    public static JsonValue parse(String text) { return parse(text, Limits.defaults()); }
    /** Parse with an explicit policy. @param text document @param limits finite policy @return immutable JSON */
    public static JsonValue parse(String text, Limits limits) { return parse(text, new Budget(limits)); }
    static JsonValue parse(String text, Budget budget) {
        Objects.requireNonNull(text);
        if (text.length() > budget.limits.maxInputBytes()) throw resource("JSON input byte ceiling");
        budget.spend(text.length());
        if (utf8Length(text) > budget.limits.maxInputBytes()) throw resource("JSON input byte ceiling");
        return parseText(text, budget);
    }
    private static JsonValue parseText(String text, Budget budget) {
        var parser = new Parser(text, budget);
        JsonValue result = parser.value(0); parser.skip();
        if (parser.pos != text.length()) throw parser.error("trailing input");
        return result;
    }
    /** Serialize under the default finite policy. @param value JSON value @return exact JSON */
    public static String stringify(JsonValue value) { return stringify(value, Limits.defaults()); }
    /** Serialize under an explicit finite policy. @param value JSON value @param limits policy @return exact JSON */
    public static String stringify(JsonValue value, Limits limits) { return stringify(value, new Budget(limits)); }
    static String stringify(JsonValue value, Budget budget) {
        var writer = new Writer(budget); writer.value(value, 0); return writer.out.toString();
    }
    /** Serialize UTF-8. @param value JSON value @return exact bytes */
    public static byte[] bytes(JsonValue value) { return stringify(value).getBytes(StandardCharsets.UTF_8); }
    static byte[] bytes(JsonValue value, Budget budget) { return stringify(value, budget).getBytes(StandardCharsets.UTF_8); }

    private static final class Writer {
        final Budget budget;
        final StringBuilder out = new StringBuilder();
        private int bytes;
        Writer(Budget budget) { this.budget = budget; }
        void append(String text, int size) {
            budget.spend(size);
            if (size > budget.limits.maxOutputBytes() - bytes) throw resource("JSON output byte ceiling");
            bytes += size; out.append(text);
        }
        void ascii(String text) { append(text, text.length()); }
        void value(JsonValue value, int depth) {
            budget.spend(1);
            if (depth > budget.limits.maxDepth()) throw resource("JSON output depth ceiling");
            switch (Objects.requireNonNull(value)) {
                case JsonNull ignored -> ascii("null");
                case JsonBoolean b -> ascii(b.value() ? "true" : "false");
                case JsonNumber n -> {
                    if (n.token.length() > budget.limits.maxNumberBytes()) throw resource("numeric token byte ceiling");
                    ascii(n.token());
                }
                case JsonString s -> quote(s.value());
                case JsonArray a -> {
                    ascii("["); boolean first = true;
                    for (JsonValue item : a.values()) { if (!first) ascii(","); first = false; value(item, depth + 1); }
                    ascii("]");
                }
                case JsonObject o -> {
                    ascii("{"); boolean first = true;
                    for (var entry : o.values().entrySet()) {
                        if (!first) ascii(","); first = false; quote(entry.getKey()); ascii(":"); value(entry.getValue(), depth + 1);
                    }
                    ascii("}");
                }
            }
        }
        void quote(String value) {
            ascii("\"");
            for (int i = 0; i < value.length(); i++) {
                char c = value.charAt(i);
                switch (c) {
                    case '"' -> ascii("\\\""); case '\\' -> ascii("\\\\");
                    case '\n' -> ascii("\\n"); case '\r' -> ascii("\\r"); case '\t' -> ascii("\\t");
                    case '\b' -> ascii("\\b"); case '\f' -> ascii("\\f");
                    default -> {
                        if (c < 0x20) ascii("\\u00" + "0123456789abcdef".charAt(c >> 4) + "0123456789abcdef".charAt(c & 15));
                        else if (Character.isHighSurrogate(c)) { append(value.substring(i, i + 2), 4); i++; }
                        else append(String.valueOf(c), c < 128 ? 1 : c < 2048 ? 2 : 3);
                    }
                }
            }
            ascii("\"");
        }
    }

    static void unicode(String text) { utf8Length(text); }
    static int utf8Length(String text) {
        Objects.requireNonNull(text);
        if (text.length() > MAX_BYTES) throw resource("Unicode string allocation ceiling");
        int bytes = 0;
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (Character.isHighSurrogate(c)) {
                if (++i >= text.length() || !Character.isLowSurrogate(text.charAt(i))) throw invalid("unpaired Unicode surrogate", i);
                bytes += 4;
            } else if (Character.isLowSurrogate(c)) throw invalid("unpaired Unicode surrogate", i);
            else bytes += c < 128 ? 1 : c < 2048 ? 2 : 3;
        }
        return bytes;
    }
    private static boolean digit(char c) { return c >= '0' && c <= '9'; }
    private static int numberEnd(String text, int pos) {
        int size = text.length();
        if (pos < size && text.charAt(pos) == '-') pos++;
        if (pos == size || !digit(text.charAt(pos))) throw invalid("invalid JSON number", pos);
        if (text.charAt(pos++) != '0') while (pos < size && digit(text.charAt(pos))) pos++;
        if (pos < size && text.charAt(pos) == '.') {
            int start = ++pos; while (pos < size && digit(text.charAt(pos))) pos++;
            if (pos == start) throw invalid("missing fraction digits", pos);
        }
        if (pos < size && (text.charAt(pos) == 'e' || text.charAt(pos) == 'E')) {
            pos++; if (pos < size && (text.charAt(pos) == '-' || text.charAt(pos) == '+')) pos++;
            int start = pos; while (pos < size && digit(text.charAt(pos))) pos++;
            if (pos == start) throw invalid("missing exponent digits", pos);
        }
        return pos;
    }
    private static final class Parser {
        private final String text;
        private final Budget budget;
        private int pos;
        Parser(String text, Budget budget) { this.text = text; this.budget = budget; }
        void skip() { while (pos < text.length() && " \t\r\n".indexOf(text.charAt(pos)) >= 0) { pos++; budget.spend(1); } }
        char at() { return pos < text.length() ? text.charAt(pos) : '\0'; }
        JsonError error(String message) { return invalid(message, pos); }
        boolean take(char c) { if (pos >= text.length() || at() != c) return false; pos++; budget.spend(1); return true; }
        JsonValue value(int depth) {
            budget.spend(1);
            if (depth > budget.limits.maxDepth()) throw resource("JSON input depth ceiling");
            skip(); char c = at();
            if (c == '"') return new JsonString(string());
            if (c == '[') {
                pos++; skip(); var values = new ArrayList<JsonValue>();
                if (!take(']')) {
                    do { values.add(value(depth + 1)); skip(); if (take(']')) return new JsonArray(values); } while (take(','));
                    throw error("expected comma or array end");
                }
                return new JsonArray(values);
            }
            if (c == '{') {
                pos++; skip(); var values = new LinkedHashMap<String, JsonValue>();
                if (!take('}')) {
                    do {
                        skip(); String key = string(); skip(); if (!take(':')) throw error("expected colon");
                        if (values.containsKey(key)) throw error("duplicate decoded object key");
                        values.put(key, value(depth + 1));
                        skip(); if (take('}')) return new JsonObject(values);
                    } while (take(','));
                    throw error("expected comma or object end");
                }
                return new JsonObject(values);
            }
            if (text.startsWith("true", pos)) { pos += 4; budget.spend(4); return new JsonBoolean(true); }
            if (text.startsWith("false", pos)) { pos += 5; budget.spend(5); return new JsonBoolean(false); }
            if (text.startsWith("null", pos)) { pos += 4; budget.spend(4); return JsonNull.INSTANCE; }
            if (c == '-' || digit(c)) {
                int start = pos; pos = numberEnd(text, pos);
                return JsonNumber.parse(text.substring(start, pos), budget);
            }
            throw error("unexpected character");
        }
        String string() {
            if (!take('"')) throw error("expected string");
            var value = new StringBuilder();
            while (pos < text.length()) {
                budget.spend(1); char c = text.charAt(pos++);
                if (c == '"') { String result = value.toString(); unicode(result); return result; }
                if (c < 0x20) throw error("unescaped control character");
                if (c == '\\') {
                    if (pos == text.length()) throw error("unterminated escape");
                    budget.spend(1); c = text.charAt(pos++);
                    switch (c) {
                        case '"', '\\', '/' -> value.append(c);
                        case 'b' -> value.append('\b'); case 'f' -> value.append('\f');
                        case 'n' -> value.append('\n'); case 'r' -> value.append('\r'); case 't' -> value.append('\t');
                        case 'u' -> {
                            int code = 0;
                            for (int i = 0; i < 4; i++) {
                                if (pos == text.length()) throw error("incomplete Unicode escape");
                                budget.spend(1); char h = text.charAt(pos++); int hex = h <= 127 ? Character.digit(h, 16) : -1;
                                if (hex < 0) throw error("invalid Unicode escape"); code = code * 16 + hex;
                            }
                            value.append((char) code);
                        }
                        default -> throw error("invalid escape");
                    }
                } else value.append(c);
            }
            throw error("unterminated string");
        }
    }
}
