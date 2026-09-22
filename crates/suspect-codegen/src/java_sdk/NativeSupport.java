import java.io.IOException;
import java.net.*;
import java.net.http.*;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import javax.net.ssl.*;

/** Independent hand-written transport seam used only by native acceptance. */
final class NativeSupport {
    private NativeSupport() {}
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    static void eventually(java.util.function.BooleanSupplier value, String message) throws InterruptedException {
        long end = System.nanoTime() + TimeUnit.SECONDS.toNanos(3);
        while (!value.getAsBoolean() && System.nanoTime() < end) Thread.sleep(2);
        check(value.getAsBoolean(), message);
    }
    record Recorded(String method, String uri, Map<String, List<String>> headers, byte[] body) {}
    static final class Script {
        final int status;
        final Map<String, List<String>> headers;
        final byte[][] chunks;
        boolean stallHeaders, stallBody, throwCancel, failBody;
        Script(int status, String media, String body) {
            this.status = status; headers = new LinkedHashMap<>();
            headers.put("Content-Type", List.of(media)); chunks = new byte[][] {body.getBytes(StandardCharsets.UTF_8)};
        }
        Script(int status, byte[][] chunks) {
            this.status = status; this.chunks = chunks; headers = new LinkedHashMap<>(); headers.put("Content-Type", List.of("application/json"));
        }
    }
    static final class Mock extends HttpClient {
        final List<Recorded> requests = Collections.synchronizedList(new ArrayList<>());
        final AtomicInteger cancellations = new AtomicInteger(), subscriptionCancellations = new AtomicInteger(), shutdowns = new AtomicInteger();
        final CountDownLatch entered = new CountDownLatch(1), bodyDelivered = new CountDownLatch(1);
        final Script script;
        Redirect redirects = Redirect.NEVER;
        CookieHandler cookies;
        Authenticator authenticator;
        Mock(Script script) { this.script = script; }
        @Override public Optional<CookieHandler> cookieHandler() { return Optional.ofNullable(cookies); }
        @Override public Optional<Duration> connectTimeout() { return Optional.empty(); }
        @Override public Redirect followRedirects() { return redirects; }
        @Override public Optional<ProxySelector> proxy() { return Optional.empty(); }
        @Override public SSLContext sslContext() { try { return SSLContext.getDefault(); } catch (java.security.NoSuchAlgorithmException error) { throw new AssertionError(error); } }
        @Override public SSLParameters sslParameters() { return new SSLParameters(); }
        @Override public Optional<Authenticator> authenticator() { return Optional.ofNullable(authenticator); }
        @Override public Version version() { return Version.HTTP_1_1; }
        @Override public Optional<Executor> executor() { return Optional.empty(); }
        @Override public void shutdownNow() { shutdowns.incrementAndGet(); }
        @Override public void close() { shutdowns.incrementAndGet(); }
        @Override public <T> HttpResponse<T> send(HttpRequest request, HttpResponse.BodyHandler<T> handler) throws IOException, InterruptedException {
            try { return sendAsync(request, handler).get(); }
            catch (ExecutionException error) { throw new IOException("mock transport failure"); }
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler, HttpResponse.PushPromiseHandler<T> push) {
            return sendAsync(request, handler);
        }
        @Override public <T> CompletableFuture<HttpResponse<T>> sendAsync(HttpRequest request, HttpResponse.BodyHandler<T> handler) {
            requests.add(new Recorded(request.method(), request.uri().toASCIIString(), request.headers().map(), requestBytes(request)));
            CompletableFuture<HttpResponse<T>> result = new CompletableFuture<>() {
                @Override public boolean cancel(boolean interrupt) {
                    cancellations.incrementAndGet();
                    if (script.throwCancel) throw new IllegalStateException("SECRET-FUTURE-CLEANUP");
                    return super.cancel(interrupt);
                }
            };
            entered.countDown();
            if (script.stallHeaders) return result;
            HttpHeaders headers = HttpHeaders.of(script.headers, (key, value) -> true);
            HttpResponse.BodySubscriber<T> subscriber = handler.apply(new HttpResponse.ResponseInfo() {
                @Override public int statusCode() { return script.status; }
                @Override public HttpHeaders headers() { return headers; }
                @Override public Version version() { return Version.HTTP_1_1; }
            });
            subscriber.getBody().whenComplete((body, error) -> {
                if (error != null) result.completeExceptionally(error);
                else result.complete(new Response<>(request, script.status, headers, body));
            });
            subscriber.onSubscribe(new Flow.Subscription() {
                private int chunk;
                private boolean completed, cancelled;
                @Override public void request(long count) {
                    if (completed || cancelled) return;
                    if (chunk < script.chunks.length) {
                        byte[] bytes = script.chunks[chunk++];
                        subscriber.onNext(List.of(ByteBuffer.wrap(bytes.clone()))); bodyDelivered.countDown();
                    } else if (!script.stallBody) {
                        completed = true;
                        if (script.failBody) subscriber.onError(new IOException("SECRET-BODY-FAILURE")); else subscriber.onComplete();
                    }
                }
                @Override public void cancel() {
                    if (!cancelled) { cancelled = true; subscriptionCancellations.incrementAndGet(); }
                    if (script.throwCancel) throw new IllegalStateException("SECRET-SUBSCRIPTION-CLEANUP");
                }
            });
            return result;
        }
    }
    private record Response<T>(HttpRequest request, int statusCode, HttpHeaders headers, T body) implements HttpResponse<T> {
        @Override public Optional<HttpResponse<T>> previousResponse() { return Optional.empty(); }
        @Override public Optional<SSLSession> sslSession() { return Optional.empty(); }
        @Override public URI uri() { return request.uri(); }
        @Override public HttpClient.Version version() { return HttpClient.Version.HTTP_1_1; }
    }
    private static byte[] requestBytes(HttpRequest request) {
        var bytes = new java.io.ByteArrayOutputStream();
        var done = new CompletableFuture<Void>();
        request.bodyPublisher().ifPresentOrElse(publisher -> publisher.subscribe(new Flow.Subscriber<ByteBuffer>() {
            @Override public void onSubscribe(Flow.Subscription subscription) { subscription.request(Long.MAX_VALUE); }
            @Override public void onNext(ByteBuffer value) { byte[] part = new byte[value.remaining()]; value.get(part); bytes.writeBytes(part); }
            @Override public void onError(Throwable error) { done.completeExceptionally(error); }
            @Override public void onComplete() { done.complete(null); }
        }), () -> done.complete(null));
        done.join(); return bytes.toByteArray();
    }
}
