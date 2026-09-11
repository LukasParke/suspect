import com.example.generated.*;
import static com.example.generated.JsonRuntime.*;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Executes shared hand-authored vectors plus independent runtime controls. */
public final class NativeValidation {
    private NativeValidation() {}
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    private static CodecException failure(String kind, Runnable action) {
        try { action.run(); } catch (CodecException error) { check(error.kind().equals(kind), "expected " + kind + ", got " + error.kind()); return error; }
        throw new AssertionError("expected " + kind);
    }
    private static void badJson(Runnable action) {
        try { action.run(); } catch (JsonRuntime.JsonError expected) { return; }
        throw new AssertionError("invalid JSON accepted");
    }
    private static JsonObject object(JsonValue value) { return (JsonObject) value; }
    private static String text(JsonValue value) { return ((JsonString) value).value(); }
    private static JsonObject replace(JsonObject value, String name, JsonValue item) {
        var fields = new LinkedHashMap<>(value.values()); fields.put(name, item); return new JsonObject(fields);
    }
    private static Validation.Program limits(JsonObject raw, String name, long value) {
        return Validation.Program.fromJson(JsonRuntime.bytes(replace(raw, "limits", replace(object(raw.values().get("limits")), name, JsonNumber.of(value)))));
    }
    private static int root(Validation.Program program, String name) {
        return program.roots().entrySet().stream().filter(e -> e.getKey().endsWith("/components/schemas/" + name)).findFirst().orElseThrow().getValue();
    }
    public static void main(String[] args) throws Exception {
        Path data = Path.of(args[0]);
        byte[] bytes = Files.readAllBytes(data.resolve("vectors-program.json"));
        Validation.Program program = Validation.Program.fromJson(bytes);
        JsonObject rawProgram = object(JsonRuntime.parse(bytes));
        JsonObject vectors = object(JsonRuntime.parse(Files.readAllBytes(data.resolve("runtime-contract-v1.json"))));
        int cases = 0, values = 0;
        for (JsonValue raw : ((JsonArray) vectors.values().get("cases")).values()) {
            JsonObject vector = object(raw); String name = text(vector.values().get("name")); int root = root(program, name); cases++;
            for (JsonValue wire : ((JsonArray) vector.values().get("valid")).values()) { program.check(root, JsonRuntime.parse(text(wire))); values++; }
            for (JsonValue wire : ((JsonArray) vector.values().get("invalid")).values()) {
                CodecException error = failure("invalid", () -> program.check(root, JsonRuntime.parse(text(wire))));
                check(error.source().contains("/components/schemas/" + name), "vector source identity lost"); values++;
            }
        }
        check(cases == 17, "shared vector count changed");
        for (String wire : List.of("", "-", "01", "+1", "1.", ".1", "1e", "1e+", "1e-", "NaN", "Infinity", "--1", "0x1", "1 ", " 1")) badJson(() -> JsonNumber.parse(wire));
        for (String wire : List.of("{}true", "[1,]", "{\"a\":1,\"\\u0061\":2}", "\"\\ud800\"", "\"\\udc00\"", "\"\n\"", "true false", "\ufeff{}", "{\"x\":1,}", "[+1]")) badJson(() -> JsonRuntime.parse(wire));
        for (byte[] wire : new byte[][] {{(byte) 0xc0, (byte) 0xaf}, {(byte) 0xed, (byte) 0xa0, (byte) 0x80}, {(byte) 0xff}, {34, (byte)0xf0, (byte)0x9f, 34}}) badJson(() -> JsonRuntime.parse(wire));
        String unicode = "雪 😀 e\u0301 \u0000 \\\"";
        check(((JsonString) JsonRuntime.parse(JsonRuntime.stringify(new JsonString(unicode)))).value().equals(unicode), "Unicode changed");
        check(JsonRuntime.parse("\"\\ud83d\\ude00\"").equals(new JsonString("😀")), "surrogate pair decoding");
        String zeroes = "0".repeat(1000);
        JsonNumber padded = JsonNumber.parse("10e-" + zeroes + "1");
        check(padded.isInteger() && padded.exactIntegerValue().intValueExact() == 1, "padded negative exponent integrality");
        check(JsonNumber.parse("1e" + zeroes).exactIntegerValue().intValueExact() == 1, "padded zero exponent conversion");
        check(!JsonNumber.parse("0.1e" + zeroes).isInteger(), "padded zero exponent rounded fraction");
        program.check(root(program, "mathematical-integers"), padded);
        failure("invalid", () -> program.check(root(program, "mathematical-integers"), JsonNumber.parse("0.1e" + zeroes)));
        JsonNumber huge = JsonNumber.parse("1e999999999999999999999999999999999999");
        check(huge.compareTo(JsonNumber.parse("9e999999999999999999999999999999999998")) > 0, "symbolic exponent comparison");
        badJson(huge::exactIntegerValue);
        badJson(() -> JsonNumber.of(0).exactIntegerValue(0));
        try { JsonNumber.parse("1.2").exactIntegerValue(); throw new AssertionError("fraction truncated"); } catch (ArithmeticException expected) { }
        check(JsonNumber.parse("100e-2").equals(JsonNumber.parse("1.00")), "exact numeric equality");
        check(JsonNumber.parse("-0").hashCode() == JsonNumber.parse("0e99999999999").hashCode(), "zero numeric hash");
        check(JsonNumber.parse("100e-2").hashCode() == JsonNumber.parse("1.00").hashCode(), "normalized numeric hash");
        check(JsonRuntime.stringify(JsonNumber.of(8), JsonRuntime.Limits.defaults().withOutputBytes(1)).equals("8"), "tight output ceiling false rejection");
        check(JsonRuntime.stringify(JsonNumber.of(99), JsonRuntime.Limits.defaults().withOutputBytes(2)).equals("99"), "tight output ceiling false rejection");
        check(JsonRuntime.stringify(new JsonString("😀"), JsonRuntime.Limits.defaults().withOutputBytes(6)).equals("\"😀\""), "UTF-8 output size");
        badJson(() -> JsonRuntime.stringify(new JsonString("😀"), JsonRuntime.Limits.defaults().withOutputBytes(5)));
        badJson(() -> JsonRuntime.parse("\"😀\"", JsonRuntime.Limits.defaults().withInputBytes(5)));
        badJson(() -> JsonRuntime.parse("[0]", JsonRuntime.Limits.defaults().withDepth(0)));
        badJson(() -> JsonRuntime.parse("[\"a\",\"b\"]", JsonRuntime.Limits.defaults().withWork(3)));
        badJson(() -> JsonRuntime.stringify(new JsonArray(List.of(new JsonString("x".repeat(10000)))), JsonRuntime.Limits.defaults().withWork(4)));
        String many = "[" + "\"a\",".repeat(32768) + "\"a\"]";
        check(((JsonArray) JsonRuntime.parse(many)).values().size() == 32769, "linear string parser control");

        Validation.Program zeroSteps = limits(rawProgram, "maxEvaluationSteps", 0);
        failure("evaluation_failure", () -> zeroSteps.check(root(zeroSteps, "boolean-true"), JsonNull.INSTANCE));
        Validation.Program zeroEquality = limits(rawProgram, "maxEqualitySteps", 0);
        failure("evaluation_failure", () -> zeroEquality.check(root(zeroEquality, "structural-numeric-equality"), JsonRuntime.parse("{\"a\":[1,null,true]}")));
        Validation.Program zeroDepth = limits(rawProgram, "maxDepth", 0);
        failure("evaluation_failure", () -> zeroDepth.check(root(zeroDepth, "boolean-true"), JsonNull.INSTANCE));
        failure("evaluation_failure", () -> program.check(root(program, "mathematical-integers"), JsonNumber.parse("1e" + "0".repeat(5000))));
        failure("evaluation_failure", () -> program.check(Integer.MAX_VALUE, JsonNull.INSTANCE));
        failure("program", () -> Validation.Program.fromJson(JsonRuntime.bytes(replace(rawProgram, "version", new JsonString("unsupported")))));
        JsonArray rawNodes = (JsonArray) rawProgram.values().get("nodes");
        var nodes = new ArrayList<>(rawNodes.values());
        for (int i = 0; i < nodes.size(); i++) {
            JsonObject node = object(nodes.get(i)); JsonArray checks = (JsonArray) node.values().get("checks");
            if (checks.values().isEmpty()) continue;
            var instructions = new ArrayList<>(checks.values()); instructions.set(0, replace(object(instructions.getFirst()), "op", new JsonString("unimplemented-noop")));
            nodes.set(i, replace(node, "checks", new JsonArray(instructions))); break;
        }
        failure("program", () -> Validation.Program.fromJson(JsonRuntime.bytes(replace(rawProgram, "nodes", new JsonArray(nodes)))));
        Validation.Program recursive = Validation.Program.fromJson(Files.readAllBytes(data.resolve("recursive-program.json")));
        for (String name : List.of("Cycle", "AnyCycle", "NotCycle")) failure("evaluation_failure", () -> recursive.check(root(recursive, name), JsonNull.INSTANCE));
        recursive.check(root(recursive, "Recursive"), JsonRuntime.parse("{\"next\":{\"next\":{}}}"));
        recursive.check(root(recursive, "UnicodePattern"), new JsonString("😀雪"));
        failure("invalid", () -> recursive.check(root(recursive, "UnicodePattern"), new JsonString("x😀雪")));
        String privateKey = "private\nFORGED\t" + "x".repeat(10000);
        CodecException diagnostic = failure("invalid", () -> program.check(root(program, "closed-object"), new JsonObject(Map.of(privateKey, JsonNull.INSTANCE))));
        check(diagnostic.instancePath().contains(privateKey), "structured diagnostic path lost");
        check(!diagnostic.getMessage().contains("\n") && !diagnostic.getMessage().contains("\t") && diagnostic.getMessage().length() < 800, "diagnostic log injection or unbounded message");
        System.out.println("JAVA_VALIDATION_OK: sharedCases=" + cases + " values=" + values + " plus grammar/Unicode/exponents/budgets/recursion/program-admission");
    }
}
