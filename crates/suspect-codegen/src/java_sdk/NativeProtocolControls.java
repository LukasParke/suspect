import example.protocol.*;
import example.protocol.Client.*;
import static example.protocol.JsonRuntime.*;
import java.net.*;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** Native control paths for the newly added protocols and stream lifetimes. */
public final class NativeProtocolControls {
    private NativeProtocolControls() {}
    static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    static SdkException fails(String kind,Runnable action){try{action.run();}catch(RuntimeException error){Throwable cause=error;while(cause instanceof CompletionException&&cause.getCause()!=null)cause=cause.getCause();if(!(cause instanceof SdkException sdk))throw new AssertionError("unclassified error",cause);check(sdk.kind().equals(kind),kind+" expected, got "+sdk.kind());check(!sdk.toString().contains("SECRET")&&sdk.getCause()==null,"unredacted failure");return sdk;}throw new AssertionError("expected "+kind);}
    static NativeSupport.Script event(String body){return new NativeSupport.Script(200,"text/event-stream",body);}
    static HttpRuntime.Options.Builder options(NativeSupport.Mock transport){return HttpRuntime.Options.builder().httpClient(transport);}
    public static void main(String[] args)throws Exception{
        var server=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","{\"ok\":true}"));
        try(var client=new Client(options(server).documentUrl(URI.create("https://docs.example.test/specs/openapi.json")).build())){
            client.serverChoice();check(server.requests.getLast().uri().equals("https://docs.example.test/v1/server"),"relative source server");
            client.serverChoice(RequestOptions.builder().serverVariable("version","v2").build());check(server.requests.getLast().uri().equals("https://docs.example.test/v2/server"),"source variable override");
            client.serverChoice(RequestOptions.builder().serverName("regional").serverVariable("region","us").serverVariable("version","v3").build());check(server.requests.getLast().uri().equals("https://us.example.test/v3/server"),"named source server");
            client.defaultServer();check(server.requests.getLast().uri().equals("https://docs.example.test/default-server"),"OAS default server provenance");
            int count=server.requests.size();fails("invalid-request",()->client.serverChoice(RequestOptions.builder().serverName("missing").build()));
            fails("invalid-request",()->client.serverChoice(RequestOptions.builder().serverVariable("version","v9").build()));
            fails("invalid-request",()->client.serverChoice(RequestOptions.builder().serverVariable("typo","v1").build()));check(server.requests.size()==count,"bad source choice reached transport");
        }
        try(var client=new Client(options(server).build())){
            fails("invalid-request",client::serverChoice);client.optionalAuth();
            check(!server.requests.getLast().headers().containsKey("Authorization"),"unconfigured optional auth");fails("missing-credential",client::oauthValue);
        }
        try(var client=new Client(options(server).credential("oauth","opaque").build())){fails("invalid-request",client::oauthValue);}
        var deadlineScript=event("data: first\n\n");deadlineScript.stallBody=true;deadlineScript.throwCancel=true;
        var deadline=new NativeSupport.Mock(deadlineScript);
        try(var client=new Client(options(deadline).timeout(Duration.ofMillis(150)).build());var response=client.events()){
            check(response.data().nextAsync().get().value().data().equals("first"),"first streaming item");
            SdkException error=fails("timeout",()->response.data().nextAsync().join());check(error.status()==200,"stream timeout lost actual status");
            NativeSupport.eventually(()->deadline.subscriptionCancellations.get()==1,"deadline did not close subscription");
        }
        var cancelScript=event("data: first\n\n");cancelScript.stallBody=true;cancelScript.throwCancel=true;
        var cancelled=new NativeSupport.Mock(cancelScript);
        try(var client=new Client(options(cancelled).build());var response=client.events()){
            check(response.data().nextAsync().get().isPresent(),"stream did not start");
            var pending=response.data().nextAsync();check(pending.cancel(true)&&pending.isCancelled(),"async pull cancellation");
            NativeSupport.eventually(()->cancelled.subscriptionCancellations.get()==1,"cancelled stream not closed");
        }
        var early=new NativeSupport.Mock(event("data: first\n\ndata: second\n\n"));
        try(var client=new Client(options(early).build());var response=client.events()){
            var done=new CompletableFuture<Void>();var delivered=new AtomicInteger();
            response.data().subscribe(new Flow.Subscriber<EventStream.Item<Event>>(){Flow.Subscription subscription;public void onSubscribe(Flow.Subscription s){subscription=s;s.request(1);}public void onNext(EventStream.Item<Event> value){delivered.incrementAndGet();subscription.cancel();done.complete(null);}public void onError(Throwable error){done.completeExceptionally(error);}public void onComplete(){done.complete(null);}});
            done.get(3,TimeUnit.SECONDS);check(delivered.get()==1,"publisher ignored demand/cancel");NativeSupport.eventually(()->early.subscriptionCancellations.get()==1,"publisher cancellation did not close body");
        }
        var overflow=new NativeSupport.Mock(event("data: "+"x".repeat(2048)+"\n\n"));overflow.script.throwCancel=true;
        try(var client=new Client(options(overflow).maxStreamBufferBytes(32).build());var response=client.events()){
            SdkException error=fails("resource-limit",()->response.data().iterator().hasNext());check(error.status()==200&&error.capture().length<=4096,"bounded stream capture");NativeSupport.eventually(()->overflow.subscriptionCancellations.get()==1,"buffer limit cleanup");
        }
        var byteLimit=new NativeSupport.Mock(event("data: first\n\n"));
        try(var client=new Client(options(byteLimit).maxResponseBytes(4).maxCaptureBytes(2).build());var response=client.events()){
            SdkException error=fails("resource-limit",()->response.data().iterator().hasNext());check(error.capture().length<=2,"response byte ceiling capture");
        }
        var invalid=new NativeSupport.Mock(new NativeSupport.Script(200,"application/x-ndjson","{\"n\":\"bad\"}\n"));
        try(var client=new Client(options(invalid).maxCaptureBytes(8).build());var response=client.lines()){
            SdkException error=fails("invalid-stream-item",()->response.data().iterator().hasNext());check(error.schemaSource().endsWith("/components/schemas/Line/properties/n/type")&&error.instancePath().equals("/n"),"stream item source binding");check(error.capture().length==8&&error.truncated(),"item failure capture");
        }
        var utf8=new NativeSupport.Mock(new NativeSupport.Script(200,new byte[][]{new byte[]{'d','a','t','a',':',' ',(byte)255,'\n','\n'}}));utf8.script.headers.put("Content-Type",List.of("text/event-stream"));
        try(var client=new Client(options(utf8).build());var response=client.events()){check(response.data().iterator().next().data().equals("\ufffd"),"SSE must use HTML replacement decoding");}
        var broken=new NativeSupport.Mock(new NativeSupport.Script(200,new byte[][]{new byte[]{(byte)255,'\n'}}));broken.script.headers.put("Content-Type",List.of("application/x-ndjson"));
        try(var client=new Client(options(broken).build());var response=client.lines()){fails("invalid-stream-item",()->response.data().iterator().hasNext());}
        var closingScript=event("data: first\n\n");closingScript.stallBody=true;var closing=new NativeSupport.Mock(closingScript);
        var client=new Client(options(closing).build());var response=client.events();check(response.data().nextAsync().get().isPresent(),"close fixture not started");client.close();
        fails("closed",()->response.data().nextAsync().join());check(closing.shutdowns.get()==0,"caller-owned HttpClient was closed");
        var stalled=new NativeSupport.Script(200,"application/json","{\"ok\":true}");stalled.stallHeaders=true;var beforeHeaders=new NativeSupport.Mock(stalled);
        try(var bound=new Client(options(beforeHeaders).build())){var pending=bound.publicValueAsync();check(beforeHeaders.entered.await(3,TimeUnit.SECONDS),"header send never entered");check(pending.cancel(true),"header future cancellation");NativeSupport.eventually(()->beforeHeaders.cancellations.get()>0,"header future transport not cancelled");}
        var wrongHeader=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","{\"ok\":true}"));wrongHeader.script.headers.put("X-Count",List.of("not-a-number"));
        try(var bound=new Client(options(wrongHeader).build())){SdkException error=fails("invalid-response-header",bound::readMetadata);check(error.status()==200,"header failure lost status");}
        var forbidden=new NativeSupport.Mock(new NativeSupport.Script(205,"application/json","null"));
        try(var bound=new Client(options(forbidden).build())){fails("unexpected-response-body",bound::unspecified);}
        var interim=new NativeSupport.Mock(new NativeSupport.Script(103,"application/json",""));
        try(var bound=new Client(options(interim).build())){try{bound.unspecified();throw new AssertionError("1xx became a success");}catch(UnspecifiedStatus103 error){check(error.status()==103&&error.data()==NoContent.INSTANCE,"1xx body disposition");}}
        var limitHeader=new NativeSupport.Mock(new NativeSupport.Script(200,"application/json","{\"ok\":true}"));limitHeader.script.headers.put("X-Large",List.of("x".repeat(1000)));
        try(var bound=new Client(options(limitHeader).maxHeaderBytes(100).build())){fails("resource-limit",bound::publicValue);}
        NativeSupport.eventually(()->Thread.getAllStackTraces().keySet().stream().noneMatch(t->t.isAlive()&&t.getName().equals("suspect-java-deadline")),"deadline thread leak");
        System.out.println("JAVA_PROTOCOL_CONTROLS_OK: source choices/hooks/header errors/stream lifetime/backpressure/deadlines/cancellation/cleanup");
    }
}
