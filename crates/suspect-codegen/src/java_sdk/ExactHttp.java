package {package};

import java.io.*;
import java.net.*;
import java.net.http.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;
import javax.net.ssl.*;

/** HTTP/1.1 fallback for method tokens the JDK treats case-insensitively (head).
 * It shares the same body subscribers, cancellation and bounds as HttpClient.
 */
final class ExactHttp {
    private ExactHttp() {}
    static <T> CompletableFuture<HttpResponse<T>> send(HttpRequest request,HttpResponse.BodyHandler<T> handler,ExecutorService executor,Duration timeout,int headerLimit,int chunkLimit){
        CompletableFuture<HttpResponse<T>> result=new CompletableFuture<>();AtomicReference<Socket> socket=new AtomicReference<>();
        result.whenComplete((value,error)->{if(result.isCancelled())close(socket.get());});
        executor.execute(()->{
            try{
                URI uri=request.uri();int port=uri.getPort()<0?(uri.getScheme().equalsIgnoreCase("https")?443:80):uri.getPort();
                Socket connection=new Socket();socket.set(connection);connection.connect(new InetSocketAddress(uri.getHost(),port),(int)Math.max(1,timeout.toMillis()));connection.setSoTimeout((int)Math.max(1,timeout.toMillis()));
                if(uri.getScheme().equalsIgnoreCase("https")){
                    SSLSocket tls=(SSLSocket)((SSLSocketFactory)SSLSocketFactory.getDefault()).createSocket(connection,uri.getHost(),port,true);
                    SSLParameters parameters=tls.getSSLParameters();parameters.setEndpointIdentificationAlgorithm("HTTPS");parameters.setApplicationProtocols(new String[]{"http/1.1"});tls.setSSLParameters(parameters);socket.set(tls);tls.startHandshake();connection=tls;
                }
                if(result.isCancelled()){close(connection);return;}
                byte[] body=requestBytes(request);String target=uri.getRawPath().isEmpty()?"/":uri.getRawPath();if(uri.getRawQuery()!=null)target+="?"+uri.getRawQuery();
                String host=uri.getRawAuthority();StringBuilder head=new StringBuilder(request.method()+" "+target+" HTTP/1.1\r\nHost: "+host+"\r\nConnection: close\r\n");
                for(var header:request.headers().map().entrySet())for(String value:header.getValue()){if(header.getKey().equalsIgnoreCase("host")||header.getKey().equalsIgnoreCase("content-length")||header.getKey().equalsIgnoreCase("connection"))throw new IllegalArgumentException("conflicting managed header");head.append(header.getKey()).append(": ").append(value).append("\r\n");}
                if(body.length>0||request.bodyPublisher().isPresent())head.append("Content-Length: ").append(body.length).append("\r\n");head.append("\r\n");
                OutputStream output=connection.getOutputStream();output.write(head.toString().getBytes(StandardCharsets.ISO_8859_1));output.write(body);output.flush();InputStream input=new BufferedInputStream(connection.getInputStream(),Math.min(chunkLimit,8192));
                int status;Map<String,List<String>> headers;int[] budget={headerLimit};
                do{
                    String first=line(input,budget);String[] pieces=first.split(" ",3);if(pieces.length<2||!Set.of("HTTP/1.0","HTTP/1.1").contains(pieces[0])||!pieces[1].matches("[1-5][0-9][0-9]"))throw new IOException("invalid HTTP status line");status=Integer.parseInt(pieces[1]);headers=new TreeMap<>(String.CASE_INSENSITIVE_ORDER);
                    for(;;){String line=line(input,budget);if(line.isEmpty())break;int colon=line.indexOf(':');if(colon<1||!HttpWire.token(line.substring(0,colon)))throw new IOException("invalid HTTP header");headers.computeIfAbsent(line.substring(0,colon),ignored->new ArrayList<>()).add(line.substring(colon+1).trim());}
                }while(status>=100&&status<200&&status!=101);
                List<String> transfer=headers.getOrDefault("transfer-encoding",List.of());boolean chunked=!transfer.isEmpty();
                if(chunked&&(transfer.size()!=1||!transfer.getFirst().equalsIgnoreCase("chunked")))throw new IOException("unsupported transfer encoding");
                List<String> lengths=headers.getOrDefault("content-length",List.of());
                if(lengths.size()>1||chunked&&!lengths.isEmpty()||!lengths.isEmpty()&&!lengths.getFirst().matches("[0-9]+"))throw new IOException("ambiguous response framing");
                long length=lengths.isEmpty()?-1:Long.parseLong(lengths.getFirst());
                final int code=status;final HttpHeaders responseHeaders=HttpHeaders.of(headers,(key,value)->true);final Socket owned=connection;
                HttpResponse.BodySubscriber<T> subscriber=handler.apply(new HttpResponse.ResponseInfo(){public int statusCode(){return code;}public HttpHeaders headers(){return responseHeaders;}public HttpClient.Version version(){return HttpClient.Version.HTTP_1_1;}});
                subscriber.getBody().whenComplete((value,error)->{if(error!=null){result.completeExceptionally(error);close(owned);}else result.complete(new Response<>(request,code,responseHeaders,value));});
                subscriber.onSubscribe(new Flow.Subscription(){volatile boolean stopped;long remaining=length,chunk;
                    public void request(long count){if(stopped)return;try{
                        if(HttpWire.forbidden(request.method(),code)){finish();return;}
                        if(chunked&&chunk==0){String size=line(input,new int[]{headerLimit});int semi=size.indexOf(';');String digits=semi<0?size:size.substring(0,semi);if(!digits.matches("[0-9A-Fa-f]+"))throw new IOException("invalid chunk length");chunk=Long.parseLong(digits,16);if(chunk==0){int[] trailers={headerLimit};while(!line(input,trailers).isEmpty()){}finish();return;}}
                        if(!chunked&&remaining==0){finish();return;}
                        int amount=(int)Math.min(Math.max(1,Math.min(chunkLimit,8192)),chunked?chunk:remaining<0?8192:remaining);byte[] bytes=new byte[amount];int countRead=input.read(bytes);
                        if(countRead<0){if(chunked||remaining>0)throw new EOFException("truncated response");finish();return;}if(countRead!=bytes.length)bytes=Arrays.copyOf(bytes,countRead);
                        if(chunked){chunk-=bytes.length;if(chunk==0&&!line(input,new int[]{2}).isEmpty())throw new IOException("invalid chunk terminator");}else if(remaining>=0)remaining-=bytes.length;
                        subscriber.onNext(List.of(ByteBuffer.wrap(bytes)));
                    }catch(Throwable error){stopped=true;close(owned);subscriber.onError(error);}}
                    private void finish(){stopped=true;close(owned);subscriber.onComplete();}
                    public void cancel(){stopped=true;close(owned);}
                });
                if(result.isCancelled())close(owned);
            }catch(Throwable error){close(socket.get());result.completeExceptionally(error);}
        });return result;
    }
    private static String line(InputStream input,int[] budget)throws IOException{
        ByteArrayOutputStream value=new ByteArrayOutputStream();for(;;){if(--budget[0]<0)throw new IOException("HTTP header ceiling");int b=input.read();if(b<0)throw new EOFException("truncated HTTP headers");if(b=='\n'){byte[] bytes=value.toByteArray();if(bytes.length==0||bytes[bytes.length-1]!='\r')throw new IOException("HTTP requires CRLF");return new String(bytes,0,bytes.length-1,StandardCharsets.ISO_8859_1);}value.write(b);}
    }
    private static byte[] requestBytes(HttpRequest request){ByteArrayOutputStream body=new ByteArrayOutputStream();CompletableFuture<Void> done=new CompletableFuture<>();request.bodyPublisher().ifPresentOrElse(p->p.subscribe(new Flow.Subscriber<ByteBuffer>(){public void onSubscribe(Flow.Subscription s){s.request(Long.MAX_VALUE);}public void onNext(ByteBuffer value){if(value.remaining()>JsonRuntime.MAX_BYTES-body.size())throw new IllegalArgumentException("request bytes ceiling");byte[] data=new byte[value.remaining()];value.get(data);body.writeBytes(data);}public void onError(Throwable e){done.completeExceptionally(e);}public void onComplete(){done.complete(null);}}),()->done.complete(null));done.join();return body.toByteArray();}
    private static void close(Socket value){if(value!=null)try{value.close();}catch(IOException ignored){}}
    private record Response<T>(HttpRequest request,int statusCode,HttpHeaders headers,T body) implements HttpResponse<T>{public Optional<HttpResponse<T>> previousResponse(){return Optional.empty();}public Optional<SSLSession> sslSession(){return Optional.empty();}public URI uri(){return request.uri();}public HttpClient.Version version(){return HttpClient.Version.HTTP_1_1;}}
}
