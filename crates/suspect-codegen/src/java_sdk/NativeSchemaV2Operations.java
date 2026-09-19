import example.schemav2.*;
import example.schemav2.Client.*;
import static example.schemav2.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import com.sun.net.httpserver.HttpServer;

/** Hand-authored source-v2 model, codec and actual HTTP expectations. */
public final class NativeSchemaV2Operations {
    private NativeSchemaV2Operations() {}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static CodecException invalid(Runnable action,String suffix){
        try{action.run();}catch(CodecException error){check(error.kind().equals("invalid"),"schema failure was reclassified: "+error.kind());check(error.source().endsWith(suffix),"wrong schema source "+error.source());return error;}throw new AssertionError("invalid source value accepted");
    }
    private static ScopedRecord.Builder record(){return ScopedRecord.builder("native").putAdditionalProperty("x-required",JsonNumber.parse("1e-400"));}
    public static void main(String[] args)throws Exception{
        var choice=ScopedChoice.decode("{\"a\":\"x\",\"b\":2}");var sequence=ScopedSequence.decode("[\"first\",2]");
        var builder=record().enabled(null).peer(null).choice(choice).sequence(sequence).putAdditionalProperty("source",new JsonBoolean(false)).putAdditionalProperty("x-integer",JsonNumber.parse("1.00"));
        var snapshot=builder.build();builder.name("later").putAdditionalProperty("x-required",JsonNumber.of(3)).omitPeer();
        check(snapshot.name().equals("native")&&snapshot.enabled().isPresent()&&snapshot.enabled().value()==null&&snapshot.peer().isPresent()&&snapshot.peer().value()==null&&!snapshot.note().isPresent(),"typed absence/null or builder snapshot changed");
        check(snapshot.kind().wireValue().equals(new JsonString("record")),"fixed literal field lost its nominal value");
        check(((JsonNumber)snapshot.additionalProperties().get("x-required")).token().equals("1e-400")&&snapshot.additionalProperties().get("source").equals(new JsonBoolean(false)),"pattern/boolean extras lost exact values or instance keys");
        try{snapshot.additionalProperties().put("x-required",JsonNull.INSTANCE);throw new AssertionError("mutable extras");}catch(UnsupportedOperationException expected){}
        check(((JsonObject)snapshot.choice().value()).values().size()==2,"all passing anyOf annotations/values were not retained");
        invalid(()->ScopedRecord.builder("x").build(),"/ScopedRecord/required");
        invalid(()->record().enabled(false).build(),"/ScopedRecord/dependentRequired/enabled");
        invalid(()->record().enabled(true).peer(null).build(),"/ScopedRecord/then/required");
        invalid(()->record().enabled(true).peer(null).note(null).build(),"/ScopedRecord/then/properties/note/type");
        invalid(()->record().putAdditionalProperty("x-integer",JsonNumber.parse("1e-400")).build(),"/ScopedRecord/patternProperties/-integer$/type");
        invalid(()->record().putAdditionalProperty("flag",new JsonString("bad")).build(),"/ScopedRecord/additionalProperties/type");
        invalid(()->record().name("x".repeat(65)).build(),"/ScopedRecord/patternProperties/^name$/maxLength");
        invalid(()->record().putAdditionalProperty("bad/name",new JsonBoolean(true)).build(),"/ScopedRecord/propertyNames/pattern");
        invalid(()->ScopedChoice.decode("{\"a\":\"x\",\"alien\":true}"),"/ScopedChoice/unevaluatedProperties");
        invalid(()->ScopedChoice.CODEC.encode(JsonRuntime.parse("{\"a\":\"x\",\"alien\":true}")),"/ScopedChoice/unevaluatedProperties");
        invalid(()->ScopedSequence.decode("[\"first\",2,3]"),"/ScopedSequence/maxContains");
        invalid(()->ScopedSequence.CODEC.encode(JsonRuntime.parse("[\"first\",1e-400]")),"/ScopedSequence/minContains");
        var node=ScopedNode.builder(JsonNumber.of(1)).next(ScopedNode.builder(JsonNumber.of(2)).build()).build();
        check(ScopedNode.decode(ScopedNode.encode(node)).next().value().value().exactIntegerValue().intValueExact()==2,"recursive scoped ref progress");
        invalid(()->ScopedNode.builder(JsonNumber.of(1)).putAdditionalProperty("alien",JsonNull.INSTANCE).build(),"/ScopedNode/unevaluatedProperties");

        String valid="{\"kind\":\"record\",\"name\":\"from-wire\",\"x-required\":1e-400,\"x-integer\":1.00,\"enabled\":null,\"peer\":null,\"source\":false,\"choice\":{\"a\":\"x\",\"b\":2},\"sequence\":[\"first\",2]}";
        var requests=Collections.synchronizedList(new ArrayList<String>());
        var server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);var executor=Executors.newVirtualThreadPerTaskExecutor();server.setExecutor(executor);
        server.createContext("/v2/",exchange->{try{
            byte[] request=exchange.getRequestBody().readAllBytes();String path=exchange.getRequestURI().getPath(),accept=exchange.getRequestHeaders().getFirst("Accept");requests.add(path);
            int status=accept.equals("error/422")?422:200;String body=path.endsWith("/read")?valid:new String(request,StandardCharsets.UTF_8);
            if(accept.equals("invalid/body"))body="{\"kind\":\"record\",\"name\":\"bad\",\"x-required\":\"not-a-number\"}";
            byte[] data=body.getBytes(StandardCharsets.UTF_8);exchange.getResponseHeaders().add("Content-Type","application/json");exchange.sendResponseHeaders(status,data.length);exchange.getResponseBody().write(data);
        }finally{exchange.close();}});server.start();
        try(var client=new Client(HttpRuntime.Options.builder().serverUrl(URI.create("http://127.0.0.1:"+server.getAddress().getPort()+"/v2")).build())){
            var echoed=client.echoScoped(EchoScopedInput.builder(snapshot).build()).data();check(ScopedRecord.encode(echoed).equals(ScopedRecord.encode(snapshot)),"scoped HTTP input/output changed data");
            var read=client.readScopedAsync().get().data();check(read.name().equals("from-wire")&&((JsonNumber)read.additionalProperties().get("x-required")).token().equals("1e-400")&&read.enabled().isPresent()&&read.enabled().value()==null,"scoped response conversion or pattern extras");
            check(client.sendSequence(SendSequenceInput.builder(sequence).build()).data().equals(sequence),"checked sequence carrier HTTP round trip");
            try{client.echoScoped(EchoScopedInput.builder(snapshot).build(),RequestOptions.builder().accept("error/422").build());throw new AssertionError("typed error became success");}
            catch(EchoScopedStatus422 error){check(error.status()==422&&error.data().name().equals("native"),"typed scoped API error");}
            try{client.readScoped(RequestOptions.builder().accept("invalid/body").build());throw new AssertionError("invalid scoped response accepted");}
            catch(SdkException error){check(error.kind().equals("invalid-response")&&error.status()==200&&error.schemaSource().endsWith("/ScopedRecord/patternProperties/^x-/type")&&error.instancePath().equals("/x-required"),"scoped response error lost its source/path");}
        }finally{server.stop(0);executor.close();}
        check(requests.size()==5,"unexpected implicit HTTP operation");
        System.out.println("JAVA_SCHEMA_V2_OPERATIONS_OK: typed fields/pattern extras, exact numbers, absence/null, immutable checked carriers, recursive refs and five actual HTTP exchanges");
    }
}
