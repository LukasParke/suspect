import com.example.generated.*;
import com.example.generated.OpenRouter.*;
import static com.example.generated.JsonRuntime.*;
import java.net.*;
import java.net.http.*;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** Independent cancellation, ownership, resource and security acceptance. */
public final class NativeTransport {
    private NativeTransport() {}
    private static final String WIDGET = "{\"id\":\"wire\",\"amount\":1.000000000000000001,\"payload\":{\"kind\":\"standard\",\"text\":\"ok\"}}";
    private static final String PAGE = "{\"items\":[]}";
    private static HttpRuntime.Options.Builder options(NativeSupport.Mock transport) {
        return HttpRuntime.Options.builder().credential("apiKey", "explicit-token==").httpClient(transport)
                .serverUrl(URI.create("https://fixture.example.test/api/v1"));
    }
    private static SdkException fails(String kind, Runnable action) {
        try { action.run(); }
        catch (RuntimeException error) {
            Throwable cause = error;
            while (cause instanceof CompletionException && cause.getCause() != null) cause = cause.getCause();
            if (!(cause instanceof SdkException sdk)) throw new AssertionError("raw runtime failure escaped", cause);
            NativeSupport.check(sdk.kind().equals(kind), kind + " expected, got " + sdk.kind());
            NativeSupport.check(sdk.getCause() == null && !sdk.toString().contains("SECRET") && !sdk.toString().contains("explicit-token"), "failure leaked cause or credentials");
            return sdk;
        }
        throw new AssertionError("expected SDK " + kind);
    }
    private static void invalid(Runnable action) {
        try { action.run(); } catch (IllegalArgumentException expected) { return; }
        throw new AssertionError("invalid configuration accepted");
    }
    public static void main(String[] args) throws Exception {
        for (String token : List.of("", "====", "a=b", "token\r\nX: secret", "雪", "Bearer value")) invalid(() -> HttpRuntime.Options.builder().credential("apiKey", token));
        for (String url : List.of("http://example.com", "https://u:p@example.com/api", "https://example.com/?x=1", "https://example.com/#x", "http://127.0.0.1:bad", "https://example.com:65536", "https://example.com:0", "https://example.com/api/..", "https://example.com/api/%2E%2e%2Fprivate", "https://example.com/%FF", "https://example.com/%5cpath")) invalid(() -> HttpRuntime.Options.builder().serverUrl(URI.create(url)).build());
        invalid(() -> HttpRuntime.Options.builder().timeout(Duration.ZERO).build());
        invalid(() -> HttpRuntime.Options.builder().maxRequestBytes(0).build());
        for (int mode = 0; mode < 3; mode++) {
            var transport = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", WIDGET));
            if (mode == 0) transport.redirects = HttpClient.Redirect.ALWAYS;
            if (mode == 1) transport.cookies = new CookieManager();
            if (mode == 2) transport.authenticator = new Authenticator() {};
            invalid(() -> options(transport).build());
        }
        var input = GetWidgetInput.builder("a/b 雪!'()*").build();
        var successful = new NativeSupport.Mock(new NativeSupport.Script(200, "Application/JSON; charset=\"utf-8\"", WIDGET));
        successful.script.headers.put("Content-Encoding", List.of("Identity"));
        try (var client = new OpenRouter(options(successful).build())) {
            var value = client.getWidget(input);
            NativeSupport.check(value.data().amount().token().equals("1.000000000000000001"), "exact response");
            var wire = successful.requests.getFirst();
            NativeSupport.check(wire.uri().equals("https://fixture.example.test/api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A"), "path bytes");
            NativeSupport.check(wire.headers().get("Authorization").equals(List.of("Bearer explicit-token==")), "source credential");
            NativeSupport.check(!wire.headers().containsKey("Cookie") && wire.body().length == 0, "implicit request state");
            try { value.headers().clear(); throw new AssertionError("mutable headers"); } catch (UnsupportedOperationException expected) { }
        }
        NativeSupport.check(successful.shutdowns.get() == 0, "SDK closed caller-owned transport");
        var queries = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", PAGE));
        var items = new ArrayList<>(List.of("a,b", "雪 +"));
        var query = ListWidgetsInput.builder().tags(items).labels(items).limit(JsonNumber.parse("2.00")).build(); items.clear();
        try (var client = new OpenRouter(options(queries).build())) {
            client.listWidgets(query);
            NativeSupport.check(queries.requests.getFirst().uri().endsWith("?tags=a%2Cb&tags=%E9%9B%AA%20%2B&labels=a%2Cb,%E9%9B%AA%20%2B&limit=2.00"), "query bytes or snapshot");
            try { query.tags().value().clear(); throw new AssertionError("mutable operation input"); } catch (UnsupportedOperationException expected) { }
        }
        var missing = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", WIDGET));
        try (var client = new OpenRouter(HttpRuntime.Options.builder().httpClient(missing).build())) {
            fails("missing-credential", () -> client.getWidget(input));
            NativeSupport.check(missing.requests.isEmpty(), "missing credential reached transport");
        }
        var limits = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", WIDGET));
        try (var client = new OpenRouter(options(limits).maxUrlBytes(80).build())) {
            fails("resource-limit", () -> client.getWidget(GetWidgetInput.builder("雪".repeat(1000)).build()));
            NativeSupport.check(limits.requests.isEmpty(), "oversized URL reached transport");
        }
        try (var client = new OpenRouter(options(limits).codecLimits(ModelCodec.Limits.defaults().withConversionSteps(20)).build())) {
            fails("resource-limit", () -> client.getWidget(input));
            NativeSupport.check(limits.requests.isEmpty(), "codec budget reached transport");
        }
        try (var client = new OpenRouter(options(limits).maxRequestBytes(64).build())) {
            fails("resource-limit", () -> client.createWidget(CreateWidgetInput.builder(WidgetInput.builder("x".repeat(1000)).build()).build()));
            NativeSupport.check(limits.requests.isEmpty(), "oversized body reached transport");
        }
        for (String media : List.of("text/plain", "application/json; charset=iso-8859-1", "application/json; broken", "application/json; charset=\"unterminated")) {
            var transport = new NativeSupport.Mock(new NativeSupport.Script(200, media, WIDGET));
            try (var client = new OpenRouter(options(transport).build())) { fails("unexpected-content-type", () -> client.getWidget(input)); }
        }
        var encoding = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", WIDGET));
        encoding.script.headers.put("Content-Encoding", List.of("gzip"));
        try (var client = new OpenRouter(options(encoding).build())) { fails("unexpected-content-encoding", () -> client.getWidget(input)); }
        for (String body : List.of("{}", "{\"id\":true}", "true false", "{\"id\":\"a\",\"\\u0069d\":\"b\"}")) {
            var transport = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", body));
            try (var client = new OpenRouter(options(transport).build())) { fails("invalid-response", () -> client.getWidget(input)); }
        }
        var unexpected = new NativeSupport.Mock(new NativeSupport.Script(302, "text/plain", "SECRET-REDIRECT"));
        unexpected.script.headers.put("Location", List.of("https://other.example/leak"));
        try (var client = new OpenRouter(options(unexpected).maxCaptureBytes(4).build())) {
            SdkException error = fails("unexpected-response", () -> client.getWidget(input));
            NativeSupport.check(error.capture().length == 4 && error.truncated() && unexpected.requests.size() == 1, "bounded unexpected response/no retry");
            byte[] capture = error.capture(); capture[0] = 0; NativeSupport.check(error.capture()[0] != 0, "capture not defensive");
        }
        var overflowScript = new NativeSupport.Script(200, new byte[][] {new byte[3], new byte[20000]}); overflowScript.throwCancel = true;
        var overflow = new NativeSupport.Mock(overflowScript);
        try (var client = new OpenRouter(options(overflow).maxResponseBytes(8).maxCaptureBytes(4).build())) {
            SdkException error = fails("resource-limit", () -> client.getWidget(input));
            NativeSupport.check(error.status() == 200 && error.capture().length == 4 && error.truncated(), "bounded body capture");
            NativeSupport.eventually(() -> overflow.subscriptionCancellations.get() == 1, "overflow subscription not cancelled");
        }
        for (boolean bodyPhase : List.of(false, true)) {
            var script = new NativeSupport.Script(200, "application/json", "{\"id\":"); script.stallHeaders = !bodyPhase; script.stallBody = bodyPhase; script.throwCancel = true;
            var transport = new NativeSupport.Mock(script);
            try (var client = new OpenRouter(options(transport).timeout(Duration.ofMillis(70)).maxCaptureBytes(3).build())) {
                long start = System.nanoTime();
                SdkException error = fails("timeout", () -> client.getWidget(input));
                NativeSupport.check(System.nanoTime() - start < TimeUnit.SECONDS.toNanos(3), "injected deadline ignored");
                NativeSupport.check(error.status() == (bodyPhase ? 200 : 0), "timeout phase/status lost");
                if (bodyPhase) NativeSupport.eventually(() -> transport.subscriptionCancellations.get() == 1, "timeout failed body cleanup");
                else NativeSupport.eventually(() -> transport.cancellations.get() > 0, "timeout failed future cancellation");
            }
        }
        var stalled = new NativeSupport.Script(200, "application/json", "{"); stalled.stallBody = true; stalled.throwCancel = true;
        var cancelled = new NativeSupport.Mock(stalled);
        try (var client = new OpenRouter(options(cancelled).build())) {
            var future = client.getWidgetAsync(input);
            NativeSupport.check(cancelled.bodyDelivered.await(3, TimeUnit.SECONDS), "body never started");
            NativeSupport.check(future.cancel(true) && future.isCancelled(), "future cancellation lost");
            NativeSupport.eventually(() -> cancelled.subscriptionCancellations.get() == 1, "cancelled body not released");
            try { future.join(); throw new AssertionError("cancelled future became a value"); } catch (CancellationException expected) { }
        }
        var syncScript = new NativeSupport.Script(200, "application/json", WIDGET); syncScript.stallHeaders = true;
        var interrupted = new NativeSupport.Mock(syncScript); var interruption = new AtomicBoolean();
        try (var client = new OpenRouter(options(interrupted).build())) {
            Thread thread = Thread.ofVirtual().start(() -> {
                SdkException error = fails("cancelled", () -> client.getWidget(input));
                interruption.set(Thread.currentThread().isInterrupted() && !error.source().isEmpty());
            });
            NativeSupport.check(interrupted.entered.await(3, TimeUnit.SECONDS), "sync send not entered");
            thread.interrupt(); thread.join(3000);
            NativeSupport.check(!thread.isAlive() && interruption.get(), "sync interrupt flag or source lost");
            NativeSupport.eventually(() -> interrupted.cancellations.get() > 0, "sync cancellation not propagated");
        }
        var closedScript = new NativeSupport.Script(200, "application/json", WIDGET); closedScript.stallHeaders = true;
        var closedTransport = new NativeSupport.Mock(closedScript);
        var closedClient = new OpenRouter(options(closedTransport).build());
        var future = closedClient.getWidgetAsync(input);
        NativeSupport.check(closedTransport.entered.await(3, TimeUnit.SECONDS), "close test not entered");
        closedClient.close(); closedClient.close();
        fails("closed", future::join); fails("closed", () -> closedClient.getWidget(input));
        NativeSupport.check(closedTransport.shutdowns.get() == 0, "close stole caller-owned transport");
        var failedScript = new NativeSupport.Script(200, "application/json", "{"); failedScript.failBody = true; failedScript.throwCancel = true;
        var failed = new NativeSupport.Mock(failedScript);
        try (var client = new OpenRouter(options(failed).build())) { fails("transport", () -> client.getWidget(input)); }
        NativeSupport.eventually(() -> Thread.getAllStackTraces().keySet().stream().noneMatch(t -> t.isAlive() && t.getName().equals("suspect-java-deadline")), "deadline threads leaked after close");
        System.out.println("JAVA_HTTP_CONTROLS_OK: wire/injection/security/limits/deadlines/cleanup/cancellation");
    }
}
