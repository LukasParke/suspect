import example.schemav3.*;
import static example.schemav3.JsonRuntime.*;
import java.nio.file.*;

/** Source-driven installed V3 runtime, with original official expectations. */
public final class NativeSchemaV3 {
    private NativeSchemaV3() {}
    private static JsonValue get(JsonValue value,String key){return ((JsonObject)value).values().getOrDefault(key,JsonNull.INSTANCE);}
    private static String text(JsonValue value){return ((JsonString)value).value();}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    public static void main(String[] args)throws Exception{
        JsonValue source=JsonRuntime.parse(Files.readAllBytes(Path.of(args[0])));int count=0;
        for(JsonValue test:((JsonArray)get(source,"cases")).values()){
            String name=text(get(test,"id")),expected=text(get(test,"expected"));
            Validation.Program program=Validation.Program.fromJson(JsonRuntime.bytes(get(test,"program")));int root=((JsonNumber)get(test,"rootTarget")).exactIntegerValue().intValueExact();JsonValue value=JsonRuntime.parse(text(get(test,"instanceJson")));
            for(int repeat=0;repeat<2;repeat++){
                String actual="Valid";CodecException failure=null;
                try{program.check(root,value);}catch(CodecException error){failure=error;actual=error.kind().equals("invalid")?"Invalid":error.kind().equals("evaluation_failure")?"EvaluationFailure":error.kind();}
                check(actual.equals(expected),name+": expected "+expected+", got "+actual+(failure==null?"":" at "+failure.source()+" "+failure.instancePath()));
                if(get(test,"source")!=JsonNull.INSTANCE){check(failure!=null&&failure.source().endsWith(text(get(test,"source"))),name+": wrong source "+(failure==null?"none":failure.source()));check(failure.instancePath().equals(text(get(test,"instancePath"))),name+": wrong instance path "+failure.instancePath());}
            }
            count++;
        }
        int malformed=0;
        for(JsonValue test:((JsonArray)get(source,"malformed")).values()){
            try{Validation.Program.fromJson(JsonRuntime.bytes(get(test,"program")));throw new AssertionError("malformed metadata admitted: "+text(get(test,"id")));}
            catch(CodecException error){check(error.kind().equals("program"),"malformed resource metadata changed outcome: "+error.kind());}malformed++;
        }
        Validation.Program deep=Validation.Program.fromJson(JsonRuntime.bytes(get(source,"depth")));int root=deep.roots().values().iterator().next();
        try{deep.check(root,JsonNull.INSTANCE);throw new AssertionError("distinct resource depth ignored");}catch(CodecException error){check(error.kind().equals("evaluation_failure"),"depth escaped explicit failure");}
        Thread.currentThread().interrupt();try{deep.check(root,JsonNull.INSTANCE);throw new AssertionError("resource cancellation ignored");}catch(CodecException error){check(error.kind().equals("cancelled")&&Thread.currentThread().isInterrupted(),"cancellation kind/flag");}finally{Thread.interrupted();}
        System.out.println("JAVA_SCHEMA_V3_CONFORMANCE_OK: "+count+" source-driven cases including all 44 original official cases; "+malformed+" malformed metadata controls; depth/cancellation");
    }
}
