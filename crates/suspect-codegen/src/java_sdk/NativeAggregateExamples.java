import example.aggregate.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import com.sun.net.httpserver.HttpServer;

/** Complete declared values checked via actual generated example call helpers. */
public final class NativeAggregateExamples {
    private NativeAggregateExamples() {}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static int count(String value,String marker){int count=0,offset=0;while((offset=value.indexOf(marker,offset))>=0){count++;offset+=marker.length();}return count;}
    public static void main(String[] args)throws Exception{
        var bodies=new ConcurrentHashMap<String,byte[]>();var server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);var executor=Executors.newVirtualThreadPerTaskExecutor();server.setExecutor(executor);
        server.createContext("/",exchange->{try{bodies.put(exchange.getRequestURI().getPath(),exchange.getRequestBody().readAllBytes());byte[] reply="true".getBytes(StandardCharsets.UTF_8);exchange.getResponseHeaders().add("Content-Type","application/json");exchange.sendResponseHeaders(200,reply.length);exchange.getResponseBody().write(reply);}finally{exchange.close();}});server.start();
        try(var client=new Client(HttpRuntime.Options.builder().serverUrl(URI.create("http://127.0.0.1:"+server.getAddress().getPort())).build())){
            check(SdkExamples.formExample(client).data(),"form example");check(SdkExamples.partsExample(client).data(),"named parts example");check(SdkExamples.sequenceExample(client).data(),"positional example");check(SdkExamples.binaryExample(client).data(),"native byte fallback example");
            check(SdkExamples.reusedExample(client).data(),"reused item schema example");check(SdkExamples.sparseExample(client).data(),"sparse prefix example");
        }finally{server.stop(0);executor.close();}
        String form=new String(bodies.get("/form"),StandardCharsets.UTF_8);check(form.equals("extra-one=first&extra-two=second&label=declared&payload=%7B%22id%22%3A7%7D"),"declared form extras/empty group/absence: "+form);
        String parts=new String(bodies.get("/parts"),StandardCharsets.UTF_8);check(count(parts,"name=\"tags\"")==2&&count(parts,"name=\"custom\"")==2&&!parts.contains("omitted"),"named declared grouping");
        for(String value:List.of("one","two","x","y"))check(parts.contains("\r\n\r\n"+value+"\r\n"),"lost declared repeated item "+value);
        String sequence=new String(bodies.get("/sequence"),StandardCharsets.UTF_8);check(count(sequence,"Content-Type: application/json")==3&&!sequence.contains("Content-Disposition"),"positional tail grouping");
        for(int n=1;n<=3;n++)check(sequence.contains("\r\n\r\n"+n+"\r\n"),"lost positional tail "+n);
        String reused=new String(bodies.get("/reused"),StandardCharsets.UTF_8);check(count(reused,"Content-Type: text/plain")==1&&count(reused,"Content-Type: application/json")==2&&reused.contains("\r\n\r\nfirst\r\n")&&reused.contains("\r\n\r\n\"second\"\r\n")&&reused.contains("\r\n\r\n\"tail\"\r\n"),"reused schema conflated wire occurrences");
        String sparse=new String(bodies.get("/sparse"),StandardCharsets.UTF_8);check(count(sparse,"Content-Type:")==1&&sparse.contains("\r\n\r\nonly\r\n"),"sparse prefix/tail invented a value");
        byte[] binary=bodies.get("/binary");boolean octets=false;for(int i=0;i<binary.length-1;i++)if(binary[i]==0&&(binary[i+1]&255)==255)octets=true;check(octets,"unavailable byte declaration became null/filename instead of labeled native octets");
        System.out.println("JAVA_DECLARED_AGGREGATE_EXAMPLES_OK: complete extras, empty optional groups, repeated values, positional tails and native byte fallback");
    }
}
