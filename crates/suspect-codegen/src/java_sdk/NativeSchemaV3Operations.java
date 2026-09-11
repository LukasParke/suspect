import example.schemav3.*;
import example.schemav3.Client.*;
import static example.schemav3.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import com.sun.net.httpserver.HttpServer;

/** Actual resource-indexed models, codec contexts and HTTP item roots. */
public final class NativeSchemaV3Operations {
    private NativeSchemaV3Operations() {}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static CodecException invalid(Runnable action,String suffix){try{action.run();}catch(CodecException error){check(error.kind().equals("invalid")&&error.source().endsWith(suffix),"wrong source outcome "+error.kind()+" "+error.source());return error;}throw new AssertionError("invalid resource value accepted");}
    public static void main(String[] args)throws Exception{
        var number=Template.builder(JsonNumber.parse("9007199254740993.000")).build();var string=Template.builder(new JsonString("text")).build();
        check(Numbers.CODEC.decode(Numbers.CODEC.encode(number)).value().equals(number.value()),"logical resource number codec");
        invalid(()->Numbers.CODEC.encode(string),"/components/schemas/Numbers/$defs/Slot/type");
        invalid(()->Strings.CODEC.encode(number),"/components/schemas/Strings/$defs/Slot/type");
        invalid(()->NumbersInput.builder(string).build(),"/components/schemas/Numbers/$defs/Slot/type");
        invalid(()->StringsInput.builder(Template.builder(new JsonString("x")).build()).build(),"/components/schemas/Strings/$defs/Slot/minLength");
        check(number.value() instanceof JsonNumber n&&n.token().equals("9007199254740993.000"),"dynamic field became a float/static fallback");
        JsonValue tree=Strict.decode("{\"data\":\"root\",\"children\":[{\"data\":\"child\"}]}");
        invalid(()->Strict.decode("{\"children\":[{\"unexpected\":1}]}"),"/components/schemas/Strict/unevaluatedProperties");
        check(Tree.decode("{\"children\":[{\"unexpected\":1}]}").children().value().size()==1,"unentered strict candidate changed generic tree");
        var children=new ArrayList<JsonValue>();children.add(JsonRuntime.parse("{\"data\":1}"));var snapshot=Tree.builder().children(children).build();children.clear();check(snapshot.children().value().size()==1,"dynamic child carrier not snapshotted");
        check(Count.CODEC.source().equals("https://physical.java.test/api.json#/components/schemas/Count"),"logical id replaced physical codec identity");
        var mock=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","{\"value\":9007199254740993}"));
        try(var client=new Client(HttpRuntime.Options.builder().httpClient(mock).build())){client.numbers(NumbersInput.builder(number).build());check(mock.requests.getLast().uri().equals("https://physical.java.test/v3/numbers"),"$self redirected the physical server base");}
        var server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);var executor=Executors.newVirtualThreadPerTaskExecutor();server.setExecutor(executor);var requests=Collections.synchronizedList(new ArrayList<String>());
        server.createContext("/v3/",exchange->{try{
            String path=exchange.getRequestURI().getPath(),accept=exchange.getRequestHeaders().getFirst("Accept");requests.add(path);byte[] request=exchange.getRequestBody().readAllBytes();String body=new String(request,StandardCharsets.UTF_8),media="application/json";int status=accept.equals("error/422")?422:200;
            if(path.endsWith("/count"))body="9007199254740993.000";
            if(path.endsWith("/number-lines")){media="application/jsonl";body="{\"value\":9007199254740993.000}\n{\"value\":\"invalid\"}\n";}
            if(accept.equals("invalid/body"))body="{\"value\":\"invalid\"}";
            byte[] bytes=body.getBytes(StandardCharsets.UTF_8);exchange.getResponseHeaders().add("Content-Type",media);exchange.sendResponseHeaders(status,bytes.length);exchange.getResponseBody().write(bytes);
        }finally{exchange.close();}});server.start();
        try(var client=new Client(HttpRuntime.Options.builder().serverUrl(URI.create("http://127.0.0.1:"+server.getAddress().getPort()+"/v3")).build())){
            check(client.numbers(NumbersInput.builder(number).build()).data().value().equals(number.value()),"dynamic integer operation");
            check(client.stringsAsync(StringsInput.builder(string).build()).get().data().value().equals(string.value()),"different dynamic caller context");
            check(client.strictTree(StrictTreeInput.builder(tree).build()).data().equals(tree),"recursive dynamic annotation operation");
            check(client.count().data().token().equals("9007199254740993.000"),"logical static ref exact response");
            try{client.numbers(NumbersInput.builder(number).build(),RequestOptions.builder().accept("error/422").build());throw new AssertionError("typed error became success");}catch(NumbersStatus422 error){check(error.status()==422&&error.data().value().equals(number.value()),"typed V3 API error");}
            try{client.numbers(NumbersInput.builder(number).build(),RequestOptions.builder().accept("invalid/body").build());throw new AssertionError("invalid dynamic response accepted");}catch(SdkException error){check(error.kind().equals("invalid-response")&&error.schemaSource().equals("https://physical.java.test/api.json#/components/schemas/Numbers/$defs/Slot/type")&&error.instancePath().equals("/value"),"physical dynamic response error identity");}
            try(var response=client.numberLines()){
                check(((JsonNumber)response.data().nextAsync().get().value().value()).token().equals("9007199254740993.000"),"resource itemSchema codec");
                try{response.data().nextAsync().join();throw new AssertionError("invalid dynamic item accepted");}catch(CompletionException error){check(error.getCause() instanceof SdkException sdk&&sdk.kind().equals("invalid-stream-item")&&sdk.schemaSource().endsWith("/components/schemas/Numbers/$defs/Slot/type")&&sdk.instancePath().equals("/value"),"dynamic item failure lost root context");}
            }
        }finally{server.stop(0);executor.close();}
        check(requests.size()==7,"unexpected implicit operation");
        System.out.println("JAVA_SCHEMA_V3_OPERATIONS_OK: logical/physical identity, typed immutable dynamic carriers, caller contexts, recursive annotations, seven HTTP exchanges and itemSchema errors");
    }
}
