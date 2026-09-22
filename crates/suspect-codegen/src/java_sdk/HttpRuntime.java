package {package};

import java.io.*;
import java.net.*;
import java.net.http.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import java.util.function.Consumer;
import java.util.function.Function;
import static {package}.JsonRuntime.*;
import static {package}.Protocol.*;

/** Source-backed HTTP with bounded bytes, native credential hooks and stream ownership. */
public final class HttpRuntime implements AutoCloseable {
    /** Caller-provided Authorization attachment; no token flow is inferred. */
    public static final class Authorization {
        private final String header;
        private Authorization(String scheme,String value){if(scheme.length()>256||!HttpWire.token(scheme)||value.isEmpty()||value.length()>16384)throw new IllegalArgumentException("invalid authorization credential");HttpWire.headerValue(value);header=scheme+" "+value;}
        /** Explicit HTTP authorization scheme and credential. @param scheme scheme @param value credential @return attachment */
        public static Authorization of(String scheme,String value){return new Authorization(Objects.requireNonNull(scheme),Objects.requireNonNull(value));}
        @Override public String toString(){return "Authorization";}
    }
    /** Immutable declared metadata passed to an OAuth/OIDC credential hook.
     * @param scheme source name
     * @param source original requirement source
     * @param metadata complete declared requirement/hook metadata
     * @param permissions declared scopes or roles
     * @param scopes true for OAuth/OIDC scopes; false for non-OAuth roles
     */
    public record CredentialContext(String scheme,String source,JsonValue metadata,List<String> permissions,boolean scopes){
        public CredentialContext {permissions=List.copyOf(permissions);Objects.requireNonNull(metadata);}
    }
    /** Runs inside the call deadline. Applications own acquisition/refresh policy. */
    @FunctionalInterface public interface CredentialProvider {
        /** Produce an explicit Authorization attachment. @param context source metadata @return credential */
        Authorization provide(CredentialContext context);
    }
    private sealed interface Credential permits Bearer,Basic,ApiKey,Hook {}
    private record Bearer(String token) implements Credential {}
    private record Basic(String user,String password) implements Credential {}
    private record ApiKey(String value) implements Credential {}
    private record Hook(CredentialProvider provider) implements Credential {}

    /** Immutable caller-owned policy and explicitly supplied source credentials. */
    public static final class Options {
        final Map<String,Credential> credentials;
        final RequestOptions choices;
        final Duration timeout;
        final int maxResponseBytes,maxRequestBytes,maxCaptureBytes,maxUrlBytes,maxHeaderBytes,maxStreamBufferBytes;
        final HttpClient httpClient;
        final ModelCodec.Limits codecLimits;
        /** Explicit ua/v1 policy: null keeps the automatic attribution header and an empty string suppresses it. */
        final String userAgent, applicationId;
        private Options(Builder b){
            credentials=Map.copyOf(b.credentials);choices=b.choices.build();timeout=b.timeout;
            maxResponseBytes=b.maxResponseBytes;maxRequestBytes=b.maxRequestBytes;maxCaptureBytes=b.maxCaptureBytes;maxUrlBytes=b.maxUrlBytes;
            maxHeaderBytes=b.maxHeaderBytes;maxStreamBufferBytes=b.maxStreamBufferBytes;httpClient=b.httpClient;codecLimits=b.codecLimits;
            userAgent=b.userAgent;applicationId=b.applicationId;
            if(timeout.isZero()||timeout.isNegative()||timeout.compareTo(Duration.ofDays(1))>0||maxResponseBytes<0||maxResponseBytes>MAX_BYTES||maxRequestBytes<0||maxRequestBytes>MAX_BYTES
                    ||maxCaptureBytes<0||maxCaptureBytes>MAX_BYTES||maxUrlBytes<1||maxUrlBytes>65536||maxHeaderBytes<1||maxHeaderBytes>MAX_BYTES||maxStreamBufferBytes<1||maxStreamBufferBytes>MAX_BYTES)
                throw new IllegalArgumentException("invalid HTTP resource policy");
            if(choices.server!=null)serverUri(choices.server);
            if(httpClient!=null&&(httpClient.followRedirects()!=HttpClient.Redirect.NEVER||httpClient.cookieHandler().isPresent()||httpClient.authenticator().isPresent()))throw new IllegalArgumentException("injected HttpClient must disable redirects, cookies and implicit authentication");
        }
        /** Begin finite options. @return builder */
        public static Builder builder(){return new Builder();}
        /** Mutable options construction. */
        public static final class Builder {
            private final Map<String,Credential> credentials=new LinkedHashMap<>();
            private final RequestOptions.Builder choices=RequestOptions.builder();
            private Duration timeout=Duration.ofSeconds(30);
            private int maxResponseBytes=MAX_BYTES,maxRequestBytes=MAX_BYTES,maxCaptureBytes=4096,maxUrlBytes=65536,maxHeaderBytes=65536,maxStreamBufferBytes=1024*1024;
            private HttpClient httpClient;
            private ModelCodec.Limits codecLimits=ModelCodec.Limits.defaults();
            private String userAgent, applicationId;
            private Builder(){}
            /** Supply a bearer credential for the exact source name. @param scheme source scheme @param token token @return builder */
            public Builder credential(String scheme,String token){Objects.requireNonNull(token);if(token.length()>16384||!token.matches("[A-Za-z0-9._~+/-]+=*"))throw new IllegalArgumentException("invalid bearer token");credentials.put(scheme(scheme),new Bearer(token));return this;}
            /** Supply HTTP Basic fields. UTF-8 is the explicit native encoding policy. @param scheme source name @param user username @param password password @return builder */
            public Builder basic(String scheme,String user,String password){JsonRuntime.unicode(user);JsonRuntime.unicode(password);if(user.indexOf(':')>=0||user.length()+password.length()>16384)throw new IllegalArgumentException("invalid basic credential");credentials.put(scheme(scheme),new Basic(user,password));return this;}
            /** Supply a source-located header/query/cookie API key. @param scheme source name @param value key @return builder */
            public Builder apiKey(String scheme,String value){JsonRuntime.unicode(value);if(value.isEmpty()||value.length()>16384)throw new IllegalArgumentException("invalid API key");credentials.put(scheme(scheme),new ApiKey(value));return this;}
            /** Supply an OAuth/OIDC attachment hook. @param scheme source name @param provider caller hook @return builder */
            public Builder authorization(String scheme,CredentialProvider provider){credentials.put(scheme(scheme),new Hook(Objects.requireNonNull(provider)));return this;}
            private static String scheme(String value){JsonRuntime.unicode(value);if(value.isEmpty()||value.length()>4096)throw new IllegalArgumentException("invalid source scheme name");return value;}
            /** Override the server explicitly. @param value URI @return builder */
            public Builder serverUrl(URI value){choices.serverUrl(value);return this;}
            /** Retrieval URL for relative servers in file-based specs. @param value HTTP URL @return builder */
            public Builder documentUrl(URI value){choices.documentUrl(value);return this;}
            /** Source server candidate. @param value index @return builder */
            public Builder serverIndex(int value){choices.serverIndex(value);return this;}
            /** Source OAS 3.2 server name. @param value name @return builder */
            public Builder serverName(String value){choices.serverName(value);return this;}
            /** Source variable override. @param name variable @param value literal value @return builder */
            public Builder serverVariable(String name,String value){choices.serverVariable(name,value);return this;}
            /** Explicit OR security choice. @param value index @return builder */
            public Builder securityAlternative(int value){choices.securityAlternative(value);return this;}
            /** Default Accept header. @param value header @return builder */
            public Builder accept(String value){choices.accept(value);return this;}
            /** Whole call/stream deadline. @param value duration @return builder */
            public Builder timeout(Duration value){timeout=Objects.requireNonNull(value);return this;}
            /** Total response/stream byte ceiling. @param value bytes @return builder */
            public Builder maxResponseBytes(int value){maxResponseBytes=value;return this;}
            /** Request byte ceiling. @param value bytes @return builder */
            public Builder maxRequestBytes(int value){maxRequestBytes=value;return this;}
            /** Explicit failure capture ceiling. @param value bytes @return builder */
            public Builder maxCaptureBytes(int value){maxCaptureBytes=value;return this;}
            /** Encoded URL byte ceiling. @param value bytes @return builder */
            public Builder maxUrlBytes(int value){maxUrlBytes=value;return this;}
            /** Header bytes, before copying native header maps. @param value bytes @return builder */
            public Builder maxHeaderBytes(int value){maxHeaderBytes=value;return this;}
            /** Maximum queued stream bytes. @param value bytes @return builder */
            public Builder maxStreamBufferBytes(int value){maxStreamBufferBytes=value;return this;}
            /** Inject a caller-owned transport. @param value client @return builder */
            public Builder httpClient(HttpClient value){httpClient=Objects.requireNonNull(value);return this;}
            /** Shared per-phase codec policy. @param value limits @return builder */
            public Builder codecLimits(ModelCodec.Limits value){codecLimits=Objects.requireNonNull(value);return this;}
            /** Full override of the automatic ua/v1 attribution header; an empty string suppresses the header entirely. @param value header value or null for automatic @return builder */
            public Builder userAgent(String value){userAgent=value;return this;}
            /** Replaces the SDK identity token in the automatic attribution header: {@code <name>} or {@code <name>/<version>} of RFC 9110 tokens. An invalid identifier omits the automatic header. @param value application identifier or null @return builder */
            public Builder applicationId(String value){applicationId=value;return this;}
            /** Check and snapshot. @return options */
            public Options build(){return new Options(this);}
        }
    }

    record Operation(JsonObject wire) {
        String source(){return Protocol.source(wire);}
        String method(){return text(wire,"method");}
    }
    record Parameter(int index,JsonValue value) {}
    record Prepared(List<Parameter> parameters,WireValue body) {}
    static Operation operation(int index){return new Operation(Protocol.object("/operations/"+index));}
    private interface Received { byte[] capture(int limit); int size(); boolean complete(); void stop(SdkException error); }
    private record Buffered(byte[] value) implements Received {
        public byte[] capture(int limit){return Arrays.copyOf(value,Math.min(limit,value.length));}
        public int size(){return value.length;}
        public boolean complete(){return true;}
        public void stop(SdkException ignored){}
    }
    private record Selection(int response,int media,boolean forbidden,String contentType,String error,boolean stream,int limit) {}
    private static Selection select(Operation op,HttpResponse.ResponseInfo info,Options options){
        int status=info.statusCode(),response=HttpWire.chooseResponse(op.wire(),status);boolean forbidden=HttpWire.forbidden(op.method(),status);String content=null,error=null;int media=-1,limit=options.maxResponseBytes;boolean stream=false;
        Map<String,List<String>> headers=info.headers().map();long headerBytes=0;for(var entry:headers.entrySet()){headerBytes+=entry.getKey().length();for(String v:entry.getValue())headerBytes+=v.length();}
        if(headerBytes>options.maxHeaderBytes)error="resource-limit";
        if(response<0)error=error==null?"unexpected-response":error;
        else {
            JsonValue declared=array(get(op.wire(),"responses")).get(response);limit=(int)Math.min(limit,number(get(declared,"max_body_bytes")));
            if(!forbidden&&!array(get(declared,"media")).isEmpty()){
                try {content=HttpWire.single(headers,"content-type",null);if(content==null)error=error==null?"unexpected-content-type":error;else media=HttpWire.chooseMedia(array(get(declared,"media")),content);}
                catch(IllegalArgumentException failure){error=error==null?"unexpected-content-type":error;}
                if(media>=0&&error==null)stream=text(get(array(get(declared,"media")).get(media),"representation"),"kind").equals("stream");
            }
        }
        List<String> encodings=headers.getOrDefault("content-encoding",List.of());
        if(!encodings.isEmpty()&&(encodings.size()!=1||!encodings.getFirst().trim().equalsIgnoreCase("identity"))){error=error==null?"unexpected-content-encoding":error;stream=false;}
        return new Selection(response,media,forbidden,content,error,stream,forbidden?0:limit);
    }
    static final class RawResponse implements AutoCloseable {
        final int status,responseIndex,mediaIndex;
        final boolean forbidden;
        final Map<String,List<String>> headers;
        final String contentType;
        final Operation operation;
        final ModelCodec.Context context;
        private final Options options;
        private final Received body;
        private final Runnable release;
        private final Executor executor;
        private final Consumer<EventStream<?>> transfer;
        private EventStream<?> eventStream;
        RawResponse(HttpResponse<Received> response,Operation op,Selection selected,Options options,Runnable release,Consumer<EventStream<?>> transfer,Executor executor){
            this.status=response.statusCode();operation=op;responseIndex=selected.response();mediaIndex=selected.media();forbidden=selected.forbidden();contentType=selected.contentType();this.options=options;body=response.body();this.release=release;this.transfer=transfer;this.executor=executor;
            if(selected.error()!=null)throw failure(selected.error());
            headers=immutableHeaders(response.headers().map());
            context=new ModelCodec.Context(options.codecLimits.withJson(options.codecLimits.json().withInputBytes(Math.min(options.maxResponseBytes,options.codecLimits.json().maxInputBytes()))));
        }
        SdkException failure(String kind){return new SdkException(kind,operation.source(),status,body.capture(options.maxCaptureBytes),!body.complete()||body.size()>options.maxCaptureBytes);}
        SdkException failure(String kind,String schema,String path){return new SdkException(kind,operation.source(),status,body.capture(options.maxCaptureBytes),!body.complete()||body.size()>options.maxCaptureBytes,schema,path);}
        Bytes bytes(){if(!(body instanceof Buffered bytes))throw failure("invalid-response");return Bytes.owned(bytes.value());}
        <T> T decode(JsonValue media,WireCodec<T> codec){
            try{return codec.read(HttpWire.decodeBody(media,bytes().internal(),contentType,context),context);}
            catch(CodecException error){throw failure(error.kind().equals("resource")||error.kind().equals("evaluation_failure")?"resource-limit":"invalid-response",error.source(),error.instancePath());}
            catch(SdkException error){throw failure(error.kind(),error.schemaSource(),error.instancePath());}
            catch(RuntimeException error){throw failure("invalid-response");}
        }
        Map<String,JsonValue> typedHeaders(JsonValue spec){try{return HttpWire.headers(spec,headers,context);}catch(SdkException e){throw failure(e.kind());}catch(RuntimeException e){throw failure("invalid-response-header");}}
        <T> EventStream<T> stream(JsonValue media,ModelCodec<T> codec){
            if(!(body instanceof StreamBuffer stream))throw failure("invalid-response");JsonValue definition=get(get(media,"representation"),"stream");
            EventStream<T> value=new EventStream<>(stream,codec,options.codecLimits,text(definition,"framing"),(int)number(get(definition,"max_item_bytes")),options.maxCaptureBytes,operation.source(),status,release,executor);
            eventStream=value;transfer.accept(value);return value;
        }
        JsonValue links(){return get(array(get(operation.wire(),"responses")).get(responseIndex),"links");}
        @Override public void close(){if(eventStream!=null)eventStream.close();else body.stop(null);release.run();}
    }

    private final Options options;
    private final HttpClient http;
    private final boolean owned;
    private final ExecutorService workers=Executors.newThreadPerTaskExecutor(Thread.ofVirtual().name("suspect-java-call-",0).factory());
    private final ScheduledThreadPoolExecutor deadlines=new ScheduledThreadPoolExecutor(1,r->{Thread t=new Thread(r,"suspect-java-deadline");t.setDaemon(true);return t;});
    private final Set<Exchange<?>> pending=ConcurrentHashMap.newKeySet();
    private final AtomicBoolean closed=new AtomicBoolean();
    /** Construct source-backed transport state. @param options immutable configuration */
    public HttpRuntime(Options options){
        this.options=Objects.requireNonNull(options);owned=options.httpClient==null;deadlines.setRemoveOnCancelPolicy(true);deadlines.setExecuteExistingDelayedTasksAfterShutdownPolicy(false);
        http=owned?HttpClient.newBuilder().connectTimeout(options.timeout).followRedirects(HttpClient.Redirect.NEVER).proxy(new ProxySelector(){public List<Proxy> select(URI uri){return List.of(Proxy.NO_PROXY);}public void connectFailed(URI uri,SocketAddress address,IOException error){}}).build():options.httpClient;
    }
    <T> CompletableFuture<T> call(Operation op,RequestOptions request,Function<ModelCodec.Context,Prepared> prepare,Function<RawResponse,T> decode){
        if(closed.get())return CompletableFuture.failedFuture(new SdkException("closed",op.source(),0,new byte[0],false));
        Exchange<T> exchange=new Exchange<>(op,Objects.requireNonNull(request),prepare,decode);pending.add(exchange);if(closed.get())exchange.abort("closed");else exchange.start();return exchange.result;
    }
    private final class Exchange<T> {
        final Operation op;final RequestOptions request;final Function<ModelCodec.Context,Prepared> prepare;final Function<RawResponse,T> decode;
        final CompletableFuture<T> result=new CompletableFuture<>();final Set<Future<?>> tasks=ConcurrentHashMap.newKeySet();final AtomicBoolean released=new AtomicBoolean();
        volatile CompletableFuture<HttpResponse<Received>> sending;volatile Received received;volatile HttpResponse.BodySubscriber<Received> subscriber;volatile Selection selection;volatile ScheduledFuture<?> timer;volatile boolean transferred;volatile int responseStatus;
        volatile EventStream<?> eventStream;volatile SdkException terminalFailure;
        final Duration timeout;
        Exchange(Operation op,RequestOptions request,Function<ModelCodec.Context,Prepared> prepare,Function<RawResponse,T> decode){this.op=op;this.request=request;this.prepare=prepare;this.decode=decode;timeout=request.timeout==null?options.timeout:request.timeout;
            result.whenComplete((v,e)->{if(result.isCancelled()){terminalFailure=error("cancelled");abortBody(terminalFailure);release();}else if(!transferred)release();});}
        void start(){try{timer=deadlines.schedule(()->abort("timeout"),timeout.toNanos(),TimeUnit.NANOSECONDS);if(released.get()){timer.cancel(false);return;}task(workers.submit(this::send));}catch(RejectedExecutionException e){abort("closed");}}
        void task(Future<?> future){tasks.add(future);if(released.get()){cancel(future);tasks.remove(future);}}
        void send(){
            try{
                if(released.get())return;
                ModelCodec.Context c=new ModelCodec.Context(options.codecLimits.withJson(options.codecLimits.json().withOutputBytes(Math.min(options.maxRequestBytes,options.codecLimits.json().maxOutputBytes()))));
                Prepared prepared=prepare.apply(c);HttpRequest outgoing=request(op,prepared,request,c,timeout);if(released.get())return;
                HttpResponse.BodyHandler<Received> handler=info->{
                    responseStatus=info.statusCode();Selection selected=select(op,info,options);selection=selected;
                    if(selected.stream()){
                        StreamBuffer stream=new StreamBuffer(info.statusCode(),op.source(),selected.limit(),options.maxStreamBufferBytes,options.maxCaptureBytes);received=stream;subscriber=stream;if(released.get())stream.stop(error("cancelled"));return stream;
                    }
                    BufferedSubscriber buffer=new BufferedSubscriber(info.statusCode(),op.source(),selected.limit(),options.maxCaptureBytes,selected.forbidden());received=buffer;subscriber=buffer;if(released.get())buffer.stop();return buffer;
                };
                CompletableFuture<HttpResponse<Received>> sent=owned&&op.method().equalsIgnoreCase("HEAD")&&!op.method().equals("HEAD")
                        ?ExactHttp.send(outgoing,handler,workers,timeout,options.maxHeaderBytes,options.maxStreamBufferBytes):http.sendAsync(outgoing,handler);
                sending=sent;if(released.get())cancel(sent);
                sent.whenComplete((response,error)->{
                    if(released.get())return;
                    if(error!=null){result.completeExceptionally(classify(error,true));return;}
                    received=response.body();
                    try{task(workers.submit(()->{
                        if(released.get())return;
                        try{T value=decode.apply(new RawResponse(response,op,selection,options,this::release,stream->{eventStream=stream;transferred=true;if(released.get())stream.abort(terminalFailure==null?error("cancelled"):terminalFailure);},workers));if(!result.complete(value)&&transferred){abortBody(terminalFailure==null?error("cancelled"):terminalFailure);release();}}
                        catch(RuntimeException failure){result.completeExceptionally(classify(failure,false));}
                    }));}catch(RejectedExecutionException failure){abort("closed");}
                });
            }catch(RuntimeException error){result.completeExceptionally(classify(error,false));}
        }
        SdkException error(String kind){Received data=received;return new SdkException(kind,op.source(),responseStatus,data==null?new byte[0]:data.capture(options.maxCaptureBytes),data!=null&&(!data.complete()||data.size()>options.maxCaptureBytes));}
        RuntimeException classify(Throwable error,boolean transport){
            if(terminalFailure!=null)return terminalFailure;Throwable cause=unwrap(error);if(cause instanceof SdkException sdk)return sdk;
            if(cause instanceof HttpTimeoutException||cause instanceof TimeoutException||cause instanceof SocketTimeoutException)return error("timeout");
            if(cause instanceof CancellationException||cause instanceof InterruptedException)return error("cancelled");
            if(cause instanceof CodecException codec)return new SdkException(codec.kind().equals("resource")||codec.kind().equals("evaluation_failure")?"resource-limit":codec.kind().equals("cancelled")?"cancelled":selection==null?"invalid-request":"invalid-response",op.source(),responseStatus,received==null?new byte[0]:received.capture(options.maxCaptureBytes),received!=null&&received.size()>options.maxCaptureBytes,codec.source(),codec.instancePath());
            if(cause instanceof JsonError json)return error(json.kind().equals("resource")?"resource-limit":selection==null?"invalid-request":"invalid-response");
            return error(transport?"transport":selection==null?"invalid-request":"invalid-response");
        }
        void abortBody(SdkException failure){EventStream<?> stream=eventStream;if(stream!=null)stream.abort(failure);Received body=received;if(body!=null)body.stop(failure);cancel(sending);for(Future<?> task:tasks)cancel(task);}
        void abort(String kind){if(released.get())return;terminalFailure=error(kind);result.completeExceptionally(terminalFailure);abortBody(terminalFailure);release();}
        void release(){if(!released.compareAndSet(false,true))return;pending.remove(this);if(timer!=null)timer.cancel(false);if(received instanceof StreamBuffer stream)stream.stop(null);cancel(sending);for(Future<?> task:tasks)if(result.isCompletedExceptionally())cancel(task);tasks.clear();}
    }
    private static void cancel(Future<?> value){if(value!=null)try{if(!value.isDone())value.cancel(true);}catch(RuntimeException ignored){}}

    private static Map<String,List<String>> immutableHeaders(Map<String,List<String>> headers){var copy=new TreeMap<String,List<String>>(String.CASE_INSENSITIVE_ORDER);headers.forEach((k,v)->copy.put(k,List.copyOf(v)));return Collections.unmodifiableMap(copy);}
    private static URI serverUri(URI uri){
        if(!uri.isAbsolute()||!("http".equalsIgnoreCase(uri.getScheme())||"https".equalsIgnoreCase(uri.getScheme()))||uri.getRawAuthority()==null||uri.getRawAuthority().contains("@")||uri.getRawQuery()!=null||uri.getRawFragment()!=null)throw new IllegalArgumentException("invalid HTTP server URI");
        if(uri.getHost()==null){
            String authority=uri.getRawAuthority(),port="";int colon=authority.lastIndexOf(':');
            if(colon>=0){port=authority.substring(colon);authority=authority.substring(0,colon);if(!port.matches(":[0-9]+"))throw new IllegalArgumentException("invalid server port");}
            String host=HttpWire.unpercent(authority,false);JsonRuntime.unicode(host);
            // java.net.IDN uses IDNA2003. Do not silently apply its transitional
            // mappings where modern HTTP URL host processing differs.
            if(host.codePoints().anyMatch(c->c==0x00df||c==0x1e9e||c==0x03c2||c==0x200c||c==0x200d))throw new IllegalArgumentException("server host needs explicit ASCII IDNA spelling");
            String ascii=IDN.toASCII(host,IDN.USE_STD3_ASCII_RULES).toLowerCase(Locale.ROOT);
            uri=URI.create(uri.getScheme()+"://"+ascii+port+(uri.getRawPath()==null?"":uri.getRawPath()));
        }
        if(uri.getHost()==null||uri.getPort()==0||uri.getPort()>65535)throw new IllegalArgumentException("invalid HTTP server authority");
        return uri.normalize();
    }
    private URI server(Operation op,RequestOptions request){
        if(request.server!=null)return serverUri(request.server);if(options.choices.server!=null)return serverUri(options.choices.server);
        List<JsonValue> candidates=array(get(get(op.wire(),"servers"),"candidates"));
        boolean overridden=request.serverName!=null||request.serverIndex!=null;
        String name=overridden?request.serverName:options.choices.serverName;Integer index=overridden?request.serverIndex:options.choices.serverIndex;int selected=index==null?0:index;
        if(name!=null){selected=-1;for(int i=0;i<candidates.size();i++){JsonValue candidate=get(candidates.get(i),"name");if(candidate!=JsonNull.INSTANCE&&text(candidate,"value").equals(name))selected=i;}}
        if(selected<0||selected>=candidates.size())throw new IllegalArgumentException("unknown server choice");JsonValue definition=candidates.get(selected);
        Map<String,String> variables=new LinkedHashMap<>(options.choices.variables);variables.putAll(request.variables);Map<String,String> expanded=new HashMap<>();
        for(JsonValue variable:array(get(definition,"variables"))){String key=text(variable,"name"),value=variables.getOrDefault(key,text(get(variable,"default"),"value"));List<JsonValue> allowed=array(get(variable,"values"));if(get(variable,"values")!=JsonNull.INSTANCE&&allowed.stream().noneMatch(v->text(v,"value").equals(value)))throw new IllegalArgumentException("server variable outside enum");expanded.put(key,value);}
        if(!expanded.keySet().containsAll(variables.keySet()))throw new IllegalArgumentException("unknown server variable");String template=text(definition,"template");StringBuilder value=new StringBuilder();
        for(int i=0;i<template.length();){if(template.charAt(i)=='{'){int end=template.indexOf('}',i+1);String variable=expanded.get(template.substring(i+1,end));if(variable==null)throw new IllegalArgumentException("missing server variable");if(variable.length()>options.maxUrlBytes-value.length())throw new IllegalArgumentException("server URL ceiling");value.append(variable);i=end+1;}else {if(value.length()>=options.maxUrlBytes)throw new IllegalArgumentException("server URL ceiling");value.append(template.charAt(i++));}}
        JsonRuntime.unicode(value.toString());
        if(value.toString().chars().anyMatch(ch->Character.isWhitespace(ch)||Character.isISOControl(ch)||"?#\\{}".indexOf(ch)>=0))throw new IllegalArgumentException("invalid server expansion");
        URI result=URI.create(value.toString());
        if(!result.isAbsolute()){
            URI document=request.documentUrl!=null?request.documentUrl:options.choices.documentUrl;
            if(document==null){String declared=Protocol.document(definition);if(declared.isEmpty())declared=op.source().split("#",2)[0];document=URI.create(declared);}
            if(!document.isAbsolute()||!("http".equalsIgnoreCase(document.getScheme())||"https".equalsIgnoreCase(document.getScheme())))throw new IllegalArgumentException("relative server needs an HTTP retrieval URL");
            result=document.resolve(result);
        }
        return serverUri(result);
    }
    private HttpRequest request(Operation op,Prepared prepared,RequestOptions request,ModelCodec.Context c,Duration timeout){
        JsonValue wire=op.wire();HttpWire.Buffer url=new HttpWire.Buffer(Math.min(options.maxUrlBytes,options.maxRequestBytes),c,wire);
        String base=server(op,request).toASCIIString();if(base.endsWith("/"))base=base.substring(0,base.length()-1);url.text(base);
        Map<String,String> paths=new HashMap<>();var query=new ArrayList<String>();var cookies=new ArrayList<String>();var headers=new TreeMap<String,String>(String.CASE_INSENSITIVE_ORDER);
        List<JsonValue> parameters=array(get(wire,"parameters"));Set<Integer> supplied=new HashSet<>();long parameterBytes=0,headerParameterBytes=0;
        for(Parameter value:prepared.parameters()){
            supplied.add(value.index());JsonValue parameter=parameters.get(value.index());String location=text(parameter,"location"),name=text(parameter,"name");
            boolean header=location.equals("header")||location.equals("cookie");
            String encoded=HttpWire.parameter(parameter,value.value(),Math.min(header?options.maxHeaderBytes:options.maxUrlBytes,options.maxRequestBytes),c);
            if(Set.of("path","query","querystring").contains(location)){parameterBytes+=encoded.length();if(parameterBytes>Math.min(options.maxUrlBytes,options.maxRequestBytes))throw new SdkException("resource-limit",op.source(),0,new byte[0],false);}
            else{headerParameterBytes+=(long)name.length()+JsonRuntime.utf8Length(encoded);if(headerParameterBytes>Math.min(options.maxHeaderBytes,options.maxRequestBytes))throw new SdkException("resource-limit",op.source(),0,new byte[0],false);}
            switch(location){case "path"->{if(encoded.equals(".")||encoded.equals(".."))throw new SdkException("invalid-path-segment",op.source(),0,new byte[0],false);paths.put(name,encoded);}case "query","querystring"->query.add(encoded);case "cookie"->cookies.add(encoded);case "header"->{HttpWire.headerValue(encoded);if(headers.putIfAbsent(name,encoded)!=null)throw new IllegalArgumentException("duplicate request header");}default->throw new IllegalArgumentException("unknown parameter location");}
        }
        for(int i=0;i<parameters.size();i++)if(flag(parameters.get(i),"required")&&!supplied.contains(i))throw new IllegalArgumentException("missing required parameter");
        String path=text(wire,"path");for(int i=0;i<path.length();){if(path.charAt(i)=='{'){int end=path.indexOf('}',i+1);String value=paths.get(path.substring(i+1,end));if(value==null)throw new IllegalArgumentException("missing path value");url.text(value);i=end+1;}else{int end=path.indexOf('{',i);if(end<0)end=path.length();HttpWire.uriLiteral(url,path.substring(i,end));i=end;}}
        attachCredentials(op,request,headers,query,cookies,c);
        for(int i=0;i<query.size();i++){url.text(i==0?"?":"&");url.text(query.get(i));}
        if(!cookies.isEmpty()){if(headers.containsKey("Cookie"))throw new IllegalArgumentException("cookie attachment conflict");headers.put("Cookie",String.join("; ",cookies));}
        String accept=request.accept!=null?request.accept:options.choices.accept;
        if(accept==null){var media=new LinkedHashSet<String>();for(JsonValue response:array(get(wire,"responses")))for(JsonValue entry:array(get(response,"media")))media.add(text(get(entry,"media_type"),"declared"));accept=media.isEmpty()?"*/*":String.join(", ",media);}
        headers.put("Accept",accept);headers.put("Accept-Encoding","identity");Bytes body=Bytes.empty();JsonValue bodyPlan=get(wire,"body");
        if(prepared.body()!=null){
            List<JsonValue> media=array(get(bodyPlan,"media"));if(media.isEmpty())throw new IllegalArgumentException("undeclared request body");WireValue value=prepared.body();String actual;JsonValue selected;
            if(value instanceof WireValue.Selected choice){actual=choice.contentType();selected=media.get(HttpWire.chooseMedia(media,actual));if(selected!=Protocol.at(choice.declaration()))throw new IllegalArgumentException("request media choice bypasses a more specific declaration");value=choice.value();}
            else{if(media.size()!=1)throw new IllegalArgumentException("request media choice required");selected=media.getFirst();actual=text(get(selected,"media_type"),"declared");HttpWire.chooseMedia(media,actual);}
            int ceiling=(int)Math.min(options.maxRequestBytes,number(get(get(bodyPlan,"limits"),"body")));HttpWire.Encoded encoded=HttpWire.encodeBody(selected,value,actual,ceiling,c);body=encoded.bytes();headers.put("Content-Type",encoded.contentType());
        }else if(bodyPlan!=JsonNull.INSTANCE&&flag(bodyPlan,"required"))throw new IllegalArgumentException("missing request body");
        // ua/v1 attribution is applied after declared parameters so an explicit
        // caller-supplied User-Agent header keeps precedence over the default.
        String userAgent=resolveUserAgent();
        if(userAgent!=null)headers.putIfAbsent("User-Agent",userAgent);
        long count=url.value().size()+body.size(),headerBytes=0;for(var header:headers.entrySet()){if(!HttpWire.token(header.getKey()))throw new IllegalArgumentException("invalid header name");HttpWire.headerValue(header.getValue());headerBytes+=header.getKey().length()+header.getValue().length();}
        if(count+headerBytes>options.maxRequestBytes||headerBytes>options.maxHeaderBytes)throw new SdkException("resource-limit",op.source(),0,new byte[0],false);
        HttpRequest.Builder builder=HttpRequest.newBuilder(URI.create(url.text())).timeout(timeout);headers.forEach(builder::header);
        return builder.method(op.method(),prepared.body()==null?HttpRequest.BodyPublishers.noBody():HttpRequest.BodyPublishers.ofByteArray(body.internal())).build();
    }
    /** ua/v1 application identity: {@code <name>} or {@code <name>/<version>} of RFC 9110 tokens. */
    static boolean applicationIdentity(String value){
        if(value.isEmpty()||value.length()>128)return false;
        int slash=value.indexOf('/');
        if(slash<0)return HttpWire.token(value);
        return value.indexOf('/',slash+1)<0&&HttpWire.token(value.substring(0,slash))&&HttpWire.token(value.substring(slash+1));
    }
    /** ua/v1 attribution: an explicit caller value wins entirely, an explicit empty value suppresses the header, and the automatic value identifies suspect as the generator and the SDK or a caller-supplied application as the client. @return header value or null for no header */
    String resolveUserAgent(){
        if(options.userAgent!=null)return options.userAgent.isEmpty()?null:options.userAgent;
        if(Attribution.SUSPECT_VERSION.isEmpty())return null;
        String identity=Attribution.SDK_NAME+"/"+Attribution.SDK_VERSION;
        if(options.applicationId!=null&&!options.applicationId.isEmpty()){
            if(!applicationIdentity(options.applicationId))return null;
            identity=options.applicationId;
        }
        String languageVersion=System.getProperty("java.version");
        if(languageVersion==null||languageVersion.isEmpty())languageVersion="unknown";
        return "suspect/"+Attribution.SUSPECT_VERSION+" "+identity+" ("+Attribution.LANGUAGE+"/"+languageVersion+"; openapi/"+Attribution.SPEC_VERSION+")";
    }
    private void attachCredentials(Operation op,RequestOptions request,Map<String,String> headers,List<String> query,List<String> cookies,ModelCodec.Context c){
        JsonValue security=get(op.wire(),"security");if(!text(security,"kind").equals("alternatives"))return;
        List<JsonValue> alternatives=array(get(security,"alternatives"));Integer forced=request.securityAlternative!=null?request.securityAlternative:options.choices.securityAlternative;JsonValue selected=null;
        if(forced!=null){if(forced>=alternatives.size())throw new IllegalArgumentException("unknown security alternative");selected=alternatives.get(forced);}
        else for(JsonValue alternative:alternatives)if(array(get(alternative,"requirements")).stream().allMatch(r->options.credentials.containsKey(text(r,"name")))){selected=alternative;break;}
        if(selected==null)throw new SdkException("missing-credential",op.source(),0,new byte[0],false);
        for(JsonValue requirement:array(get(selected,"requirements"))){
            String name=text(requirement,"name");Credential credential=options.credentials.get(name);if(credential==null)throw new SdkException("missing-credential",op.source(),0,new byte[0],false);
            JsonValue hook=get(requirement,"credential");String kind=text(hook,"kind"),header=null;
            if(kind.equals("bearer")&&credential instanceof Bearer bearer)header="Bearer "+bearer.token();
            else if(kind.equals("basic")&&credential instanceof Basic basic)header="Basic "+Base64.getEncoder().encodeToString((basic.user()+":"+basic.password()).getBytes(StandardCharsets.UTF_8));
            else if(kind.equals("api-key")&&credential instanceof ApiKey key){
                String wireName=text(get(hook,"name"),"value"),location=text(hook,"location");c.spend(key.value().length());
                if(location.equals("header")){HttpWire.headerValue(key.value());if(headers.putIfAbsent(wireName,key.value())!=null)throw new IllegalArgumentException("credential header conflict");}
                else if(location.equals("query")){String encodedName=HttpWire.percent(wireName,"uri-component","query",null,false,options.maxUrlBytes,c,requirement);for(String part:query)for(String pair:part.split("&"))if(pair.startsWith(encodedName+"="))throw new IllegalArgumentException("credential query conflict");query.add(encodedName+"="+HttpWire.percent(key.value(),"uri-component","query",null,false,options.maxUrlBytes,c,requirement));}
                else{HttpWire.cookieValue(key.value());for(String value:cookies)if(value.startsWith(wireName+"="))throw new IllegalArgumentException("credential cookie conflict");cookies.add(wireName+"="+key.value());}continue;
            }else if((kind.equals("o-auth2")||kind.equals("open-id-connect"))&&credential instanceof Hook provider){
                JsonValue permissions=get(requirement,"permissions");List<String> names=array(get(permissions,"names")).stream().map(p->text(p,"value")).toList();
                Authorization authorization=provider.provider().provide(new CredentialContext(name,Protocol.source(requirement),requirement,names,text(permissions,"kind").equals("scopes")));
                if(authorization==null)throw new SdkException("missing-credential",op.source(),0,new byte[0],false);header=authorization.header;
            }else throw new IllegalArgumentException("credential does not match declared attachment kind");
            c.spend(header.length());if(headers.putIfAbsent("Authorization",header)!=null)throw new IllegalArgumentException("authorization conflict");
        }
    }
    static Throwable unwrap(Throwable error){while((error instanceof CompletionException||error instanceof ExecutionException)&&error.getCause()!=null)error=error.getCause();return error;}
    static <T> T await(CompletableFuture<T> future,String source){try{return future.get();}catch(InterruptedException error){cancel(future);Thread.currentThread().interrupt();throw new SdkException("cancelled",source,0,new byte[0],false);}catch(CancellationException error){throw new SdkException("cancelled",source,0,new byte[0],false);}catch(ExecutionException error){Throwable cause=unwrap(error);if(cause instanceof SdkException sdk)throw sdk;throw new SdkException("transport",source,0,new byte[0],false);}}

    private static final class BufferedSubscriber implements Received,HttpResponse.BodySubscriber<Received>{
        final int status,limit,capture;final String source;final boolean forbidden;final ByteArrayOutputStream bytes=new ByteArrayOutputStream();final CompletableFuture<Received> result=new CompletableFuture<>();
        volatile Flow.Subscription subscription;final AtomicBoolean cancelled=new AtomicBoolean();final AtomicInteger requests=new AtomicInteger();
        BufferedSubscriber(int status,String source,int limit,int capture,boolean forbidden){this.status=status;this.source=source;this.limit=limit;this.capture=capture;this.forbidden=forbidden;}
        public CompletionStage<Received> getBody(){return result;}
        public void onSubscribe(Flow.Subscription value){if(subscription!=null){safeCancel(value);return;}subscription=value;if(result.isDone())stop();else more();}
        void more(){if(requests.getAndIncrement()!=0)return;int n=1;do{if(!result.isDone())try{subscription.request(1);}catch(RuntimeException error){onError(error);}n=requests.addAndGet(-n);}while(n!=0);}
        public synchronized void onNext(List<ByteBuffer> values){if(result.isDone())return;try{for(ByteBuffer buffer:values){int count=Math.min(buffer.remaining(),limit-bytes.size());byte[] part=new byte[count];buffer.get(part);bytes.writeBytes(part);if(buffer.hasRemaining()){result.completeExceptionally(new SdkException(forbidden?"unexpected-response-body":"resource-limit",source,status,capture(capture),true));stop();return;}}more();}catch(RuntimeException error){onError(error);}}
        public void onError(Throwable error){Throwable cause=unwrap(error);result.completeExceptionally(cause instanceof SdkException?cause:new SdkException(cause instanceof HttpTimeoutException||cause instanceof SocketTimeoutException?"timeout":cause instanceof CancellationException?"cancelled":"transport",source,status,capture(capture),true));stop();}
        public synchronized void onComplete(){result.complete(new Buffered(bytes.toByteArray()));}
        public synchronized byte[] capture(int limit){byte[] prefix=bytes.toByteArray();return Arrays.copyOf(prefix,Math.min(limit,prefix.length));}
        public synchronized int size(){return bytes.size();}
        public boolean complete(){return result.isDone()&&!result.isCompletedExceptionally();}
        public void stop(SdkException failure){if(failure==null)result.completeExceptionally(new CancellationException());else result.completeExceptionally(failure);Flow.Subscription value=subscription;if(value!=null&&cancelled.compareAndSet(false,true))safeCancel(value);}
        void stop(){stop(null);}
    }
    private static void safeCancel(Flow.Subscription subscription){try{subscription.cancel();}catch(RuntimeException ignored){}}
    private static final class StreamBuffer extends InputStream implements Received,HttpResponse.BodySubscriber<Received>{
        final int status,totalLimit,bufferLimit,captureLimit;final String source;final ArrayDeque<byte[]> queue=new ArrayDeque<>();final ByteArrayOutputStream prefix=new ByteArrayOutputStream();
        final CompletableFuture<Received> result=CompletableFuture.completedFuture(this);final AtomicBoolean cancelled=new AtomicBoolean();volatile Flow.Subscription subscription;
        int queued,total,offset;boolean ended,waiting;SdkException error;
        StreamBuffer(int status,String source,int totalLimit,int bufferLimit,int captureLimit){this.status=status;this.source=source;this.totalLimit=totalLimit;this.bufferLimit=bufferLimit;this.captureLimit=captureLimit;}
        public CompletionStage<Received> getBody(){return result;}
        public void onSubscribe(Flow.Subscription value){synchronized(this){if(subscription!=null){safeCancel(value);return;}subscription=value;notifyAll();}if(ended)safeCancel(value);}
        public void onNext(List<ByteBuffer> buffers){
            SdkException failure=null;synchronized(this){waiting=false;if(ended)return;long count=0;for(ByteBuffer b:buffers)count+=b.remaining();if(count>totalLimit-total||count>bufferLimit-queued)failure=new SdkException("resource-limit",source,status,capture(captureLimit),true);
                else for(ByteBuffer b:buffers){byte[] value=new byte[b.remaining()];b.get(value);queue.add(value);queued+=value.length;total+=value.length;if(prefix.size()<captureLimit)prefix.write(value,0,Math.min(value.length,captureLimit-prefix.size()));}notifyAll();}
            if(failure!=null)stop(failure);
        }
        public void onError(Throwable cause){stop(new SdkException(cause instanceof HttpTimeoutException||cause instanceof SocketTimeoutException?"timeout":"transport",source,status,capture(captureLimit),true));}
        public synchronized void onComplete(){waiting=false;ended=true;notifyAll();}
        @Override public int read() throws IOException{
            for(;;){Flow.Subscription request=null;synchronized(this){if(error!=null)throw error;while(!queue.isEmpty()&&offset==queue.peek().length){queue.remove();offset=0;}if(!queue.isEmpty()){queued--;return queue.peek()[offset++]&255;}if(ended)return -1;
                    if(subscription!=null&&!waiting){waiting=true;request=subscription;}else try{wait();}catch(InterruptedException interrupted){Thread.currentThread().interrupt();stop(new SdkException("cancelled",source,status,capture(captureLimit),true));throw new IOException("interrupted");}}
                if(request!=null)try{request.request(1);}catch(RuntimeException failure){stop(new SdkException("transport",source,status,capture(captureLimit),true));}
            }
        }
        public synchronized byte[] capture(int limit){byte[] bytes=prefix.toByteArray();return Arrays.copyOf(bytes,Math.min(bytes.length,limit));}
        public synchronized int size(){return total;}
        public synchronized boolean complete(){return ended&&error==null;}
        public void stop(SdkException failure){Flow.Subscription value;synchronized(this){if(error==null&&failure!=null)error=failure;ended=true;queue.clear();queued=0;offset=0;value=subscription;notifyAll();}if(value!=null&&cancelled.compareAndSet(false,true))safeCancel(value);}
        @Override public void close(){stop(null);}
    }
    /** Cancel all calls and streams; injected transports retain caller ownership. */
    @Override public void close(){if(!closed.compareAndSet(false,true))return;for(Exchange<?> exchange:pending)exchange.abort("closed");deadlines.shutdownNow();workers.shutdownNow();if(owned)http.shutdownNow();}
}
