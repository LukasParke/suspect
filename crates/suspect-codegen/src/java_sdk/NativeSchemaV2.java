import example.schemav2.*;
import static example.schemav2.JsonRuntime.*;
import java.nio.file.*;
import java.util.*;

/** Maintained schema source vectors run through the actual installed Java executor. */
public final class NativeSchemaV2 {
    private NativeSchemaV2() {}
    private static JsonValue get(JsonValue object, String key) { return ((JsonObject)object).values().getOrDefault(key, JsonNull.INSTANCE); }
    private static String text(JsonValue value) { return ((JsonString)value).value(); }
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    private static String path(String id) {
        return switch(id) {
            case "contains-zero-does-not-mark-unmatched", "contains-exact-integrality" -> "/0";
            case "contains-failure-after-exceeded-maximum" -> "/1";
            case "pattern-overlap-rejects", "named-and-pattern-both-apply" -> "/x";
            case "property-names-checks-key-not-value" -> "/long";
            case "property-names-does-not-annotate-values" -> "/ok";
            case "failed-anyof-branch-does-not-leak", "allof-cousins-have-independent-scopes", "not-discards-annotations", "required-is-not-an-evaluation" -> "/a";
            case "nested-members-do-not-mark-parent" -> "/inner";
            case "prefix-and-contains-leave-unmatched-item" -> "/2";
            default -> "";
        };
    }
    public static void main(String[] args) throws Exception {
        JsonValue document=JsonRuntime.parse(Files.readAllBytes(Path.of(args[0]))); int count=0;
        for(JsonValue test:((JsonArray)get(document,"cases")).values()) {
            String id=text(get(test,"id")), expected=text(get(test,"expected"));
            Validation.Program program=Validation.Program.fromJson(JsonRuntime.bytes(get(test,"program")));
            int root=program.roots().values().iterator().next(); JsonValue instance=JsonRuntime.parse(text(get(test,"instanceJson")));
            for(int repeat=0;repeat<2;repeat++) {
                String actual="Valid"; CodecException failure=null;
                try { program.check(root,instance); }
                catch(CodecException error) { failure=error; actual=error.kind().equals("invalid")?"Invalid":error.kind().equals("evaluation_failure")?"EvaluationFailure":error.kind(); }
                check(actual.equals(expected),id+": expected "+expected+", got "+actual+(failure==null?"":" at "+failure.source()+" "+failure.instancePath()));
                if(get(test,"source")!=JsonNull.INSTANCE) {
                    check(failure!=null&&failure.source().endsWith(text(get(test,"source"))),id+": wrong source "+(failure==null?"none":failure.source()));
                    String location=get(test,"instancePath")==JsonNull.INSTANCE?path(id):text(get(test,"instancePath"));
                    check(failure.instancePath().equals(location),id+": wrong instance location "+failure.instancePath());
                }
            }
            count++;
        }
        int rejected=0;
        for(JsonValue test:((JsonArray)get(document,"malformed")).values()) {
            try { Validation.Program.fromJson(JsonRuntime.bytes(get(test,"program"))); throw new AssertionError("malformed program admitted: "+text(get(test,"id"))); }
            catch(CodecException error) { check(error.kind().equals("program"),"malformed program became validation outcome"); }
            rejected++;
        }
        JsonValue recursive=((JsonArray)get(document,"cases")).values().stream().filter(v->text(get(v,"id")).equals("recursive-ref-annotations-with-instance-progress")).findFirst().orElseThrow();
        Validation.Program program=Validation.Program.fromJson(JsonRuntime.bytes(get(recursive,"program")));int root=program.roots().values().iterator().next();
        JsonValue deep=new JsonObject(Map.of());for(int i=0;i<200;i++)deep=new JsonObject(Map.of("next",deep));
        try { program.check(root,deep); throw new AssertionError("depth ceiling ignored"); }
        catch(CodecException error) { check(error.kind().equals("evaluation_failure"),"depth was not a bounded evaluation failure"); }
        Thread.currentThread().interrupt();
        try { program.check(root,new JsonObject(Map.of())); throw new AssertionError("cancellation ignored"); }
        catch(CodecException error) { check(error.kind().equals("cancelled")&&Thread.currentThread().isInterrupted(),"cancellation was inverted or interrupt flag lost"); }
        finally { Thread.interrupted(); }
        System.out.println("JAVA_SCHEMA_V2_VECTORS_OK: "+count+" source-driven cases; "+rejected+" malformed programs; exact outcomes/locations, fresh scopes, depth and cancellation");
    }
}
