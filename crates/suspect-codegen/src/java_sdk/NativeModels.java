import com.example.generated.*;
import static com.example.generated.JsonRuntime.*;
import java.util.*;

/** Independent mutation, null, union, literal and recursion regressions. */
public final class NativeModels {
    private NativeModels() {}
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    private static CodecException fails(Runnable action) {
        try { action.run(); } catch (CodecException expected) { return expected; }
        throw new AssertionError("invalid native value accepted");
    }
    private static void resource(Runnable action) {
        CodecException error = fails(action);
        check(error.kind().equals("resource") || error.kind().equals("evaluation_failure"), "resource exhaustion changed into " + error.kind());
    }
    public static void main(String[] args) {
        var absent = PresenceCase.builder(null, "ready").build();
        check(absent.requiredNullable() == null && !absent.optionalNullable().isPresent() && !absent.optionalString().isPresent(), "absence/null collapsed");
        String encoded = PresenceCase.encode(absent);
        check(encoded.contains("\"required_nullable\":null") && !encoded.contains("optional_nullable"), "required null or absent field lost");
        var present = PresenceCase.builder(null, "ready").optionalNullable(null).raw(JsonNull.INSTANCE).build();
        check(present.optionalNullable().isPresent() && present.optionalNullable().value() == null, "present null lost");
        check(present.raw().value() == JsonNull.INSTANCE, "JSON native null changed representation");
        fails(() -> PresenceCase.builder(null, null).build());
        fails(() -> PresenceCase.builder(null, "ready").optionalString(null).build());
        fails(() -> PresenceCase.builder(null, "ready").amount(null).build());
        fails(() -> PresenceCase.builder(null, "ready").raw(null).build());
        fails(() -> PresenceCase.decode("{\"required_string\":\"ready\",\"tag\":\"fixed\"}"));
        check(PresenceCase.builder(null, "ready").optionalNullable("x").omitOptionalNullable().build().equals(absent), "omit setter changed value");
        check(absent.tag().equals(PresenceCaseTag.FIXED), "source-proved fixed tag");
        check(NullableObject.decode("null") == null, "nullable object root");
        check(NullableHolder.builder(null).build().value() == null, "nullable reference model");
        check(NullableHolder.encode(NullableHolder.builder(null).build()).equals("{\"value\":null}"), "nullable reference encode");
        check(RequiredJson.builder(JsonNull.INSTANCE).build().anything() == JsonNull.INSTANCE, "required JSON null");
        fails(() -> RequiredJson.builder(null).build());

        var mutable = new ArrayList<String>(Arrays.asList("original", null));
        var builder = PresenceCase.builder(null, "ready").items(mutable);
        var frozen = builder.build(); mutable.set(0, "changed"); builder.optionalString("later");
        check(frozen.items().value().equals(Arrays.asList("original", null)) && !frozen.optionalString().isPresent(), "builder/list mutation leaked");
        try { frozen.items().value().clear(); throw new AssertionError("model list is mutable"); } catch (UnsupportedOperationException expected) { }
        var rawMap = new LinkedHashMap<String, JsonValue>();
        var rawList = new ArrayList<JsonValue>(); rawList.add(new JsonString("original"));
        rawMap.put("nested", new JsonArray(rawList));
        var raw = new JsonObject(rawMap); rawMap.clear(); rawList.clear();
        var extra = PresenceCase.builder(null, "ready").putAdditionalProperty("extra", raw).build();
        check(PresenceCase.encode(extra).contains("original"), "generic JSON snapshot was shallow");
        try { extra.additionalProperties().clear(); throw new AssertionError("extras are mutable"); } catch (UnsupportedOperationException expected) { }
        try { ((JsonObject) extra.additionalProperties().get("extra")).values().clear(); throw new AssertionError("nested JSON is mutable"); } catch (UnsupportedOperationException expected) { }
        try { PresenceCase.builder(null, "ready").putAdditionalProperty("required_string", new JsonString("override")); throw new AssertionError("declared key overwritten by extra"); } catch (IllegalArgumentException expected) { }

        for (String wire : List.of("\"auto\"", "true", "1.00", "null", "{\"a\":[1e0]}")) {
            var value = MixedLiteral.decode(wire);
            check(JsonRuntime.parse(MixedLiteral.encode(value)).equals(JsonRuntime.parse(wire)), "mixed literal round trip " + wire);
        }
        check(NumberEnum.encode(NumberEnum.decode("1.000")).equals("1.000"), "numeric literal spelling was canonicalized");
        check(NumberEnum.decode("1.00").equals(NumberEnum.decode("1e0")), "mathematical literal equality");
        check(NumberEnum.decode("1.00").hashCode() == NumberEnum.decode("1e0").hashCode(), "literal hash mismatch");
        fails(() -> NumberEnum.decode("2"));
        fails(() -> MixedLiteral.decode("false"));
        for (String wire : List.of("\"auto\"", "{\"name\":\"ok\",\"other\":[1.0,null]}")) {
            var value = MixedUnion.decode(wire);
            check(JsonRuntime.parse(MixedUnion.encode(value)).equals(JsonRuntime.parse(wire)), "literal/object union conversion failed");
        }
        check(MixedUnion.decode("{\"name\":\"ok\"}") instanceof MixedUnion.Variant2, "wrong object arm");
        fails(() -> new StringUnion.Variant1("x"));
        fails(() -> new StringUnion.Variant2("long"));
        var arm = new StringUnion.Variant1("long");
        check(StringUnion.decode(StringUnion.encode(arm)) instanceof StringUnion.Variant1, "selected arm changed");
        fails(() -> Overlap.encode(new Overlap.Variant1(JsonNumber.of(1))));
        fails(() -> Overlap.decode("1.0"));
        check(Overlap.decode("1.5") instanceof Overlap.Variant2, "exclusive numeric union");
        var nullArm = NullUnion.decode("null");
        check(nullArm instanceof NullUnion.Variant1 && NullUnion.decode(NullUnion.encode(nullArm)) instanceof NullUnion.Variant1, "null arm lost");
        fails(() -> NullUnion.encode(null));
        var inclusive = Inclusive.decode("{\"a\":\"x\",\"b\":1e0}");
        check(inclusive instanceof Inclusive.Variant1, "anyOf first matching view");
        check(JsonRuntime.parse(Inclusive.encode(inclusive)).equals(JsonRuntime.parse("{\"a\":\"x\",\"b\":1.0}")), "inclusive union dropped another arm's fields");
        var recursive = Recursive.decode("{\"name\":\"root\",\"child\":{\"name\":\"leaf\"}}");
        check(!recursive.child().value().child().isPresent(), "recursive absence lost");
        check(Recursive.decode(Recursive.encode(recursive)).equals(recursive), "recursive object identity");
        var nested = RecursiveUnion.decode("[[\"a\"],\"b\"]");
        check(RecursiveUnion.encode(nested).equals("[[\"a\"],\"b\"]"), "recursive union failed");
        var unionItems = new ArrayList<RecursiveUnion>(); unionItems.add(new RecursiveUnion.Variant1("a"));
        var union = new RecursiveUnion.Variant2(unionItems); unionItems.clear();
        check(union.value().size() == 1, "union constructor did not snapshot");
        try { union.value().clear(); throw new AssertionError("union payload mutable"); } catch (UnsupportedOperationException expected) { }
        var ledger = Ledger.builder().putAdditionalProperty("a/b~雪", JsonNumber.parse("1e-400")).build();
        check(ledger.additionalProperties().get("a/b~雪").token().equals("1e-400"), "typed extras");
        fails(() -> Ledger.builder().putAdditionalProperty("bad", null).build());

        resource(() -> StringUnion.CODEC.withLimits(ModelCodec.Limits.defaults().withEvaluationSteps(10)).decode("\"long\""));
        resource(() -> NumberEnum.CODEC.withLimits(ModelCodec.Limits.defaults().withEqualitySteps(0)).decode("1"));
        resource(() -> MixedUnion.CODEC.withLimits(ModelCodec.Limits.defaults().withEqualitySteps(1)).encode(new MixedUnion.Variant1(MixedUnionVariant1.AUTO)));
        resource(() -> PresenceCase.CODEC.withLimits(ModelCodec.Limits.defaults().withConversionSteps(80)).encode(PresenceCase.builder(null, "ready").putAdditionalProperty("big", new JsonString("x".repeat(65536))).build()));
        resource(() -> PresenceCase.CODEC.withLimits(ModelCodec.Limits.defaults().withConversionSteps(80)).encode(PresenceCase.builder(null, "ready").optionalString("x".repeat(65536)).build()));
        resource(() -> PresenceCase.CODEC.withLimits(ModelCodec.Limits.defaults().withJson(JsonRuntime.Limits.defaults().withOutputBytes(4))).encode(absent));
        resource(() -> PresenceCase.CODEC.withLimits(ModelCodec.Limits.defaults().withConversionDepth(0)).encode(absent));
        resource(() -> Recursive.decode("{\"name\":\"x\",\"child\":".repeat(200) + "{\"name\":\"leaf\"}" + "}".repeat(200)));
        CodecException mismatch = fails(() -> PresenceCase.decode("{\"required_nullable\":null,\"required_string\":12,\"tag\":\"fixed\"}"));
        check(mismatch.source().endsWith("/properties/required_string/type") && mismatch.instancePath().equals("/required_string"), "codec source/path lost");
        System.out.println("JAVA_MODEL_CONTROLS_OK: presence/mutation/literals/unions/recursion/exactness/shared-budgets");
    }
}
