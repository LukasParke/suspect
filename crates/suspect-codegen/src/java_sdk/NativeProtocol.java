import example.protocol.*;
import example.protocol.Client.*;
import static example.protocol.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import com.sun.net.httpserver.HttpServer;

/** Hand-authored byte expectations for the rich Java protocol adapter. */
public final class NativeProtocol {
    private NativeProtocol() {}
    record Request(String method,String uri,Map<String,List<String>> headers,byte[] body) {
        String header(String name){return headers.entrySet().stream().filter(e->e.getKey().equalsIgnoreCase(name)).map(e->e.getValue().getFirst()).findFirst().orElse("");}
        String text(){return new String(body,StandardCharsets.UTF_8);}
    }
    private static final List<Request> REQUESTS=Collections.synchronizedList(new ArrayList<>());
    private static Request last(){return REQUESTS.getLast();}
    private static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    private static SdkException fails(String kind,Runnable action){
        try{action.run();}catch(RuntimeException error){Throwable cause=error;while(cause instanceof CompletionException&&cause.getCause()!=null)cause=cause.getCause();if(!(cause instanceof SdkException sdk))throw new AssertionError("raw failure",cause);check(sdk.kind().equals(kind),"expected "+kind+", got "+sdk.kind());check(!sdk.toString().contains("secret")&&sdk.getCause()==null,"unredacted failure");return sdk;}
        throw new AssertionError("expected "+kind);
    }
    private static RequestOptions accept(String value){return RequestOptions.builder().accept(value).build();}
    private static byte[] utf8(String value){return value.getBytes(StandardCharsets.UTF_8);}
    private static byte[] namedParts(){
        var out=new java.io.ByteArrayOutputStream();out.writeBytes(utf8("preamble\r\n--wire-boundary\r\nContent-Disposition: form-data; name=\"file\"; filename=\"wire.bin\"\r\nContent-Type: application/octet-stream\r\nX-Size: 2\r\n\r\n"));out.writeBytes(new byte[]{0,(byte)255});out.writeBytes(utf8("\r\n--wire-boundary\r\nContent-Disposition: form-data; name=\"title\"\r\nContent-Type: text/plain\r\n\r\nwire title\r\n--wire-boundary\r\nContent-Disposition: form-data; name=\"tags\"\r\nContent-Type: text/plain\r\n\r\na\r\n--wire-boundary\r\nContent-Disposition: form-data; name=\"tags\"\r\nContent-Type: text/plain\r\n\r\nb\r\n--wire-boundary--\r\nepilogue"));return out.toByteArray();
    }
    private static byte[] positional(){var out=new java.io.ByteArrayOutputStream();out.writeBytes(utf8("--position\r\nContent-Type: text/plain\r\n\r\nprefix\r\n--position\r\nContent-Type: application/octet-stream\r\n\r\n"));out.writeBytes(new byte[]{0,(byte)255});out.writeBytes(utf8("\r\n--position\r\nContent-Type: application/json\r\n\r\n{\"n\":3}\r\n--position--\r\n"));return out.toByteArray();}
    public static void main(String[] args) throws Exception {
        HttpServer server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);var executor=Executors.newVirtualThreadPerTaskExecutor();server.setExecutor(executor);
        server.createContext("/",exchange->{
            try{
                String uri=exchange.getRequestURI().toASCIIString(),path=exchange.getRequestURI().getPath();String accept=exchange.getRequestHeaders().getFirst("Accept");
                REQUESTS.add(new Request(exchange.getRequestMethod(),uri,Map.copyOf(exchange.getRequestHeaders()),exchange.getRequestBody().readAllBytes()));
                int status=200;String media="application/json";byte[] body=utf8("{\"ok\":true}");
                if(path.endsWith("/choose")){
                    switch(accept){
                        case "application/json;profile=Exact" -> {media=accept;body=utf8("{\"name\":\"profile\",\"amount\":1e-400}");}
                        case "text/plain" -> {media=accept;body=utf8("10e-000000000000000000000000000000001");}
                        case "application/octet-stream","image/png","application/pdf" -> {media=accept;body=new byte[]{0,(byte)255};}
                        case "range/207" -> {status=207;media="text/plain";body=utf8("range");}
                        case "invalid/json" -> body=utf8("{\"name\":\"cannot-bypass-json\"}");
                        case "missing/type" -> media=null;
                        default -> { }
                    }
                }else if(path.endsWith("/fallback")){status=Integer.parseInt(accept.substring(accept.lastIndexOf('/')+1));media="text/plain";body=utf8("fallback");}
                else if(path.endsWith("/unspecified")){status=Integer.parseInt(accept.substring(accept.lastIndexOf('/')+1));media="application/octet-stream";body=new byte[]{0,1,(byte)255};}
                else if(path.endsWith("/unknown")){status=299;media="application/octet-stream";body=utf8("unknown secret response");}
                else if(path.endsWith("/metadata")){exchange.getResponseHeaders().add("X-Count",accept.equals("bad/header")?"bad":"10e-1");exchange.getResponseHeaders().add("X-Flags","true,false");exchange.getResponseHeaders().add("X-Color","B=150,G=200,R=100");exchange.getResponseHeaders().add("X-JSON","{\"name\":\"header\",\"amount\":1e-400}");}
                else if(path.endsWith("/head")){exchange.getResponseHeaders().add("X-Count","2");}
                else if(path.endsWith("/form-response")){media="application/x-www-form-urlencoded";body=utf8("title=hello+%E9%9B%AA&values=1.0&values=2&payload=%7B%22name%22%3A%22from-form%22%7D&codes=a%2Cb&codes=c");}
                else if(path.endsWith("/parts")){media="multipart/form-data; boundary=wire-boundary";body=namedParts();}
                else if(path.endsWith("/positional-response")){media="multipart/mixed;boundary=position";body=positional();}
                else if(path.endsWith("/events")){media="text/event-stream";body=utf8("\uFEFF: comment\r\nid: one\r\nevent: update\r\nretry: 0015\r\ndata: snow 雪\r\ndata: {\"n\":1}\r\n\r\n: heartbeat\n\nid: bad\0id\nretry: invalid\ndata: [DONE]\n\ndata: incomplete");}
                else if(path.endsWith("/lines")){media="application/x-ndjson";body=utf8("{\"n\":1,\"exact\":1e-400}\n{\"n\":2}");}
                else if(path.endsWith("/nullable-lines")){media="application/jsonl";body=utf8("null\n\"x\"\nnull\n");}
                else if(path.endsWith("/legacy30-file")){media="application/octet-stream";body=new byte[]{0,(byte)255};}
                if(media!=null)exchange.getResponseHeaders().add("Content-Type",media);
                exchange.getResponseHeaders().add("Set-Cookie","secret=session; Path=/");
                boolean forbidden=exchange.getRequestMethod().equals("HEAD")||status<200||status==204||status==205||status==304;
                exchange.sendResponseHeaders(status,forbidden?-1:body.length);
                if(!forbidden)exchange.getResponseBody().write(body);
            }finally{exchange.close();}
        });server.start();URI base=URI.create("http://127.0.0.1:"+server.getAddress().getPort()+"/api/v1");
        try(var client=new Client(HttpRuntime.Options.builder().serverUrl(base).credential("bearer","secret-token").basic("basic","user","pass")
                .apiKey("headerKey","header-secret").apiKey("queryKey","query +/secret").apiKey("cookieKey","cookie-secret")
                .authorization("oauth",context->{check(context.scopes()&&context.permissions().equals(List.of("read")),"OAuth permissions");check(context.metadata().toString().contains("token"),"OAuth metadata");return HttpRuntime.Authorization.of("Custom","oauth-secret");})
                .authorization("oidc",context->{check(context.permissions().equals(List.of("openid")),"OIDC permissions");return HttpRuntime.Authorization.of("Custom","oidc-secret");}).build())){
            check(client.publicValue().data().ok(),"anonymous request");check(last().header("Authorization").isEmpty(),"anonymous operation attached credentials");
            client.optionalAuth();check(last().header("Authorization").equals("Bearer secret-token"),"optional configured auth");
            client.optionalAuth(RequestOptions.builder().securityAlternative(1).build());check(last().header("Authorization").isEmpty(),"explicit anonymous alternative");
            client.andAuth();check(last().header("X-Key").equals("header-secret")&&last().header("Cookie").equals("session=cookie-secret")&&last().uri().endsWith("/and?key=query%20%2B%2Fsecret"),"AND API key attachment");
            client.orAuth();check(last().header("Authorization").equals("Basic dXNlcjpwYXNz"),"Basic explicit fields");
            client.orAuth(RequestOptions.builder().securityAlternative(1).build());check(last().header("Authorization").equals("Bearer secret-token"),"OR choice");
            client.oauthValue();check(last().header("Authorization").equals("Custom oauth-secret"),"OAuth attachment hook");
            client.oidcValue();check(last().header("Authorization").equals("Custom oidc-secret"),"OIDC attachment hook");
            var color=Color.builder(JsonNumber.of(150),JsonNumber.of(200),JsonNumber.of(100)).build();
            client.styles(StylesInput.builder(List.of("blue","black"),color,"a/b 雪").color(color).multi(List.of("a,b","c")).spaces(List.of("a","b")).pipes(List.of("a","b")).filter(color)
                .reserved("https://example.test/a?x%3D%2B").content(Payload.builder("payload").build()).xFlag(true).xTags(List.of("a","b")).sid("a b").crumb(List.of("x","y")).build());
            check(last().uri().equals("/api/v1/styles/.blue.black/;B=150;G=200;R=100/a%2Fb%20%E9%9B%AA?color=B,150,G,200,R,100&multi=a%2Cb&multi=c&spaces=a%20b&pipes=a%7Cb&filter%5BB%5D=150&filter%5BG%5D=200&filter%5BR%5D=100&reserved=https://example.test/a?x%3D%2B&content=%7B%22name%22%3A%22payload%22%7D"),"style URI: "+last().uri());
            check(last().header("X-Flag").equals("true")&&last().header("X-Tags").equals("a,b")&&last().header("Cookie").equals("sid=a%20b; crumb=x; crumb=y"),"header/cookie styles");
            client.wholeQuery(WholeQueryInput.builder(Payload.builder("x").build()).build());check(last().uri().equals("/api/v1/whole?%7B%22name%22%3A%22x%22%7D"),"whole query JSON");
            client.wholeText(WholeTextInput.builder("a=b & 雪%2F").build());check(last().uri().endsWith("?a%3Db%20%26%20%E9%9B%AA%252F"),"whole query text");
            client.wholeForm(WholeFormInput.builder(WholeFormParametersValue0ApplicationXWwwFormUrlencoded.builder(true,"a + b").build()).build());check(last().uri().endsWith("?flag=true&foo=a+%2B+b"),"whole query form encoded twice");
            var selected=(ChooseResponseStatus200)client.chooseResponse(accept("application/json"));check(selected.data() instanceof ChooseResponseResponse200Body.Json json&&json.value().ok(),"JSON media");
            selected=(ChooseResponseStatus200)client.chooseResponse(accept("application/json;profile=Exact"));check(selected.data() instanceof ChooseResponseResponse200Body.JsonExact json&&json.value().amount().value().token().equals("1e-400"),"parameterized JSON media");
            selected=(ChooseResponseStatus200)client.chooseResponse(accept("text/plain"));check(selected.data() instanceof ChooseResponseResponse200Body.Text text&&text.value().exactIntegerValue().intValueExact()==1,"exact text scalar");
            selected=(ChooseResponseStatus200)client.chooseResponse(accept("application/octet-stream"));check(selected.data() instanceof ChooseResponseResponse200Body.Binary2 bytes&&Arrays.equals(bytes.value().toByteArray(),new byte[]{0,(byte)255}),"binary media");
            selected=(ChooseResponseStatus200)client.chooseResponse(accept("image/png"));check(selected.data() instanceof ChooseResponseResponse200Body.ImageRange,"type wildcard media");
            selected=(ChooseResponseStatus200)client.chooseResponse(accept("application/pdf"));check(selected.data() instanceof ChooseResponseResponse200Body.Binary,"wildcard media");
            var range=(ChooseResponseStatus2XX)client.chooseResponse(accept("range/207"));check(range.status()==207&&range.data().value().equals("range"),"actual range status");
            fails("invalid-response",()->client.chooseResponse(accept("invalid/json")));fails("unexpected-content-type",()->client.chooseResponse(accept("missing/type")));
            var fallback=client.defaultResponse(accept("status/201"));check(fallback.status()==201&&fallback.data().value().equals("fallback"),"default 201 misclassified");
            var empty=client.defaultResponse(accept("status/204"));check(empty.status()==204&&!empty.data().isPresent(),"default body-forbidden status");
            try{client.defaultResponse(accept("status/418"));throw new AssertionError("default error became success");}catch(DefaultResponseStatusDefaultError error){check(error.status()==418&&error.data().value().equals("fallback"),"default typed failure");}
            var raw=(UnspecifiedStatus200)client.unspecified(accept("status/200"));check(Arrays.equals(raw.data().toByteArray(),new byte[]{0,1,(byte)255}),"undeclared body bytes");
            check(((UnspecifiedStatus204)client.unspecified(accept("status/204"))).data()==NoContent.INSTANCE,"204 none");
            check(((UnspecifiedStatus205)client.unspecified(accept("status/205"))).data()==NoContent.INSTANCE,"205 none");
            try{client.unspecified(accept("status/304"));throw new AssertionError();}catch(UnspecifiedStatus304 error){check(error.data()==NoContent.INSTANCE,"304 none");}
            fails("unexpected-response",client::undeclaredResponses);
            var metadata=client.readMetadata();check(metadata.typedHeaders().xCount().exactIntegerValue().intValueExact()==1,"header mathematical integer");
            check(metadata.typedHeaders().xFlags().value().equals(List.of(true,false)),"header array");check(metadata.typedHeaders().xColor().value().r().exactIntegerValue().intValueExact()==100,"header object");
            check(metadata.typedHeaders().xJson().value().name().equals("header")&&((JsonArray)metadata.links()).values().size()==1,"header JSON/link metadata");
            check(client.headValue().data()==NoContent.INSTANCE&&last().method().equals("HEAD"),"HEAD body disposition");
            client.putMethod();check(last().method().equals("PUT"),"PUT");client.deleteMethod();check(last().method().equals("DELETE"),"DELETE");client.optionsMethod();check(last().method().equals("OPTIONS"),"OPTIONS");client.traceMethod();check(last().method().equals("TRACE"),"TRACE");client.queryMethod();check(last().method().equals("QUERY"),"QUERY");
            client.copyMethod();check(last().method().equals("COPY"),"COPY");client.mixedGet();check(last().method().equals("GeT"),"case-preserved GeT");client.lowerGet();check(last().method().equals("get"),"case-preserved get");client.lowerHead();check(last().method().equals("head")&&last().body().length==0,"lowercase head");client.customPing();check(last().method().equals("x-PING"),"custom token");
            client.sendBody(SendBodyInput.builder(new SendBodyBody.Json(Payload.builder("native").amount(JsonNumber.parse("1e-400")).build())).build());check(last().text().equals("{\"amount\":1e-400,\"name\":\"native\"}"),"native JSON body");
            client.sendBody(SendBodyInput.builder(new SendBodyBody.Text(JsonNumber.parse("1.00"))).build());check(last().text().equals("1.00"),"native text body");
            client.sendBody(SendBodyInput.builder(new SendBodyBody.Binary(Bytes.of(new byte[]{0,(byte)255}),"application/octet-stream")).build());check(Arrays.equals(last().body(),new byte[]{0,(byte)255}),"native byte body");
            int before=REQUESTS.size();fails("invalid-request",()->client.sendBody(SendBodyInput.builder(new SendBodyBody.Binary(Bytes.empty(),"application/json")).build()));check(REQUESTS.size()==before,"wildcard bypass reached transport");
            var form=SubmitFormForm.builder("hello 雪").values(List.of(JsonNumber.parse("1.0"),JsonNumber.of(2))).payload(Payload.builder("form").build()).codes(List.of("a,b","c")).build();
            client.submitForm(SubmitFormInput.builder(form).build());check(last().text().equals("codes=a%2Cb&codes=c&payload=%7B%22name%22%3A%22form%22%7D&title=hello+%E9%9B%AA&values=1.0&values=2"),"OAS 3.2 applies field encoding to each array item: "+last().text());
            var readForm=client.readForm().data();check(readForm.title().equals("hello 雪")&&readForm.values().value().getFirst().token().equals("1.0")&&readForm.codes().value().equals(List.of("a,b","c")),"form response decoding");
            var file=UploadMultipartFilePart.builder(Bytes.of(new byte[]{0,(byte)255}),UploadMultipartFileHeaders.builder(JsonNumber.of(2)).build()).filename("native.bin").build();
            var upload=UploadMultipart.builder(file,UploadMultipartTitlePart.builder("title").build()).tags(List.of(UploadMultipartTagsPart.builder("a").build(),UploadMultipartTagsPart.builder("b").build())).payload(UploadMultipartPayloadPart.builder(Payload.builder("metadata").build()).build()).build();
            client.upload(UploadInput.builder(upload).build());check(last().header("Content-Type").startsWith("multipart/form-data; boundary="),"multipart boundary");check(last().text().contains("name=\"file\"; filename=\"native.bin\"")&&last().text().contains("X-Size: 2\r\n"),"part header metadata");check(last().body()[last().text().indexOf("\r\n\r\n")+4]==0,"file octets");
            var parts=client.downloadParts().data();check(Arrays.equals(parts.file().value().toByteArray(),new byte[]{0,(byte)255})&&parts.file().filename().equals("wire.bin")&&parts.tags().value().size()==2,"multipart response");
            var positional=SendPositionalMultipart.builder(SendPositionalMultipartPart1Part.builder("prefix").build()).part2(SendPositionalMultipartPart2Part.builder(Bytes.of(new byte[]{0,(byte)255})).build()).addItem(SendPositionalMultipartAdditionalPart.builder(Line.builder(JsonNumber.of(3)).build()).build()).build();
            client.sendPositional(SendPositionalInput.builder(positional).build());check(!last().text().contains("Content-Disposition"),"positional parts acquired fake names");
            var positions=client.readPositional().data();check(positions.part1().value().equals("prefix")&&Arrays.equals(positions.part2().value().value().toByteArray(),new byte[]{0,(byte)255})&&positions.items().getFirst().value().n().exactIntegerValue().intValueExact()==3,"positional response");
            client.sendLines(SendLinesInput.builder(List.of(Line.builder(JsonNumber.of(1)).exact(JsonNumber.parse("1e-400")).build(),Line.builder(JsonNumber.of(2)).build())).build());check(last().text().equals("{\"exact\":1e-400,\"n\":1}\n{\"n\":2}\n"),"source item request framing");
            client.legacy30(Legacy30Input.builder(LegacyBase.builder("name","string-ref").amount(null).build()).build());check(last().text().equals("{\"amount\":null,\"name\":\"name\",\"note\":\"string-ref\"}"),"external 3.0 context/ref siblings/nullable");
            try{LegacyBase.builder("name","note").amount(JsonNumber.of(0)).build();throw new AssertionError("3.0 exclusive minimum ignored");}catch(CodecException expected){}
            check(Arrays.equals(client.legacy30File().data().toByteArray(),new byte[]{0,(byte)255}),"external 3.0 binary must use octets without legacy opt-in");
            client.form31(Form31Input.builder(Form31Form.builder(List.of("a,b","c")).build()).build());check(last().text().equals("codes=a%2Cb,c"),"3.1 whole-array style confused with 3.2 per-item encoding");
            try(var response=client.events()){
                var events=new ArrayList<Event>();for(Event event:response.data())events.add(event);
                check(events.size()==2,"SSE emitted comment, heartbeat, sentinel or incomplete block");
                check(events.getFirst().data().equals("snow 雪\n{\"n\":1}")&&events.getFirst().event().value().equals("update")&&events.getFirst().retry().value().exactIntegerValue().intValueExact()==15,"parsed SSE envelope");
                check(events.getLast().data().equals("[DONE]")&&events.getLast().id().value().equals("one")&&!events.getLast().retry().isPresent(),"SSE state/sentinel");
            }
            try(var response=client.linesAsync().get()){
                var stream=response.data();check(stream.nextAsync().get().value().exact().value().token().equals("1e-400"),"async line item");check(stream.nextAsync().get().value().n().exactIntegerValue().intValueExact()==2,"JSON-lines EOF item");check(!stream.nextAsync().get().isPresent(),"line EOF");
            }
            try(var response=client.nullableLines()){
                var items=new ArrayList<String>();for(String item:response.data())items.add(item);check(items.equals(Arrays.asList(null,"x",null)),"JSON null collapsed into stream EOF");
            }
            try(var response=client.nullableLines()){
                var done=new CompletableFuture<Void>();var items=Collections.synchronizedList(new ArrayList<String>());
                response.data().subscribe(new Flow.Subscriber<EventStream.Item<String>>(){Flow.Subscription subscription;public void onSubscribe(Flow.Subscription value){subscription=value;value.request(1);}public void onNext(EventStream.Item<String> value){items.add(value.value());subscription.request(1);}public void onError(Throwable error){done.completeExceptionally(error);}public void onComplete(){done.complete(null);}});
                done.get(5,TimeUnit.SECONDS);check(items.equals(Arrays.asList(null,"x",null)),"Flow null item/backpressure");
            }
            check(REQUESTS.stream().noneMatch(r->r.header("Cookie").contains("secret=session")),"response cookie persisted");
        }finally{server.stop(0);executor.close();}
        System.out.println("JAVA_PROTOCOL_WIRE_OK: "+REQUESTS.size()+" independent real HTTP exchanges");
    }
}
