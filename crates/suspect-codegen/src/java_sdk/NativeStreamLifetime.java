import example.protocol.*;
import example.protocol.Client.*;
import static example.protocol.JsonRuntime.*;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** Additional streaming terminals and source-declared finite request framing. */
public final class NativeStreamLifetime {
    private NativeStreamLifetime() {}
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }
    private static SdkException fails(String kind, Runnable action) { return NativeProtocolControls.fails(kind, action); }
    private static HttpRuntime.Options.Builder options(NativeSupport.Mock transport) { return HttpRuntime.Options.builder().httpClient(transport); }
    private static NativeSupport.Mock stalled() {
        var script = new NativeSupport.Script(200, "text/event-stream", "data: later\n\n"); script.stallBody = true;
        return new NativeSupport.Mock(script);
    }
    private static final class Waiting implements Flow.Subscriber<EventStream.Item<Event>> {
        final CompletableFuture<String> terminal = new CompletableFuture<>();
        public void onSubscribe(Flow.Subscription subscription) { }
        public void onNext(EventStream.Item<Event> item) { terminal.completeExceptionally(new AssertionError("item without demand")); }
        public void onError(Throwable error) { terminal.complete(error instanceof SdkException sdk ? sdk.kind() : error.getClass().getName()); }
        public void onComplete() { terminal.complete("complete"); }
    }
    private static void zeroDemand() throws Exception {
        var transport = stalled();
        try (var client = new Client(options(transport).timeout(Duration.ofMillis(150)).build()); var response = client.events()) {
            var waiting = new Waiting(); response.data().subscribe(waiting);
            check(waiting.terminal.get(3, TimeUnit.SECONDS).equals("timeout"), "zero-demand publisher missed deadline");
            check(transport.bodyDelivered.getCount() == 1 && transport.subscriptionCancellations.get() == 1, "zero demand prefetched or leaked bytes");
        }
        var closing = stalled(); var client = new Client(options(closing).build()); var response = client.events();
        var waiting = new Waiting(); response.data().subscribe(waiting); client.close();
        check(waiting.terminal.get(3, TimeUnit.SECONDS).equals("closed") && closing.shutdowns.get() == 0, "client-close terminal/ownership");
        var early = stalled();
        try (var bound = new Client(options(early).build())) {
            var value = bound.events(); var subscriber = new Waiting(); value.data().subscribe(subscriber); value.close();
            check(subscriber.terminal.get(3, TimeUnit.SECONDS).equals("complete"), "explicit response close did not complete publisher");
        }
    }
    private static void throwingSubscriber() throws Exception {
        var transport = stalled();
        try (var client = new Client(options(transport).build()); var response = client.events()) {
            response.data().subscribe(new Flow.Subscriber<EventStream.Item<Event>>() {
                public void onSubscribe(Flow.Subscription subscription) { throw new IllegalStateException("user callback"); }
                public void onNext(EventStream.Item<Event> item) { throw new AssertionError("unsubscribed callback"); }
                public void onError(Throwable error) { throw new AssertionError("unsubscribed callback"); }
                public void onComplete() { throw new AssertionError("unsubscribed callback"); }
            });
            NativeSupport.eventually(() -> transport.subscriptionCancellations.get() == 1, "throwing subscriber leaked body");
        }
        var pull = stalled();
        try (var client = new Client(options(pull).build()); var response = client.events()) {
            check(response.data().nextAsync().get().isPresent(), "first async item");
            try { response.data().iterator(); throw new AssertionError("mixed stream consumers"); } catch (IllegalStateException expected) { }
            var pending = response.data().nextAsync();
            try { response.data().nextAsync(); throw new AssertionError("concurrent pulls"); } catch (IllegalStateException expected) { }
            pending.cancel(true);
        }
    }
    private static void finiteRequests() {
        var transport = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", "{\"ok\":true}"));
        try (var client = new Client(options(transport).build())) {
            client.sendEvents(SendEventsInput.builder(List.of(Event.builder(" first\n\nlast").id(" id").event(" update").retry(JsonNumber.parse("20e-1")).build(), Event.builder("[DONE]").id(" id").build())).build());
            check(new String(transport.requests.getLast().body(), StandardCharsets.UTF_8).equals("id:  id\nevent:  update\nretry: 2\ndata:  first\ndata: \ndata: last\n\nid:  id\ndata: [DONE]\n\n"), "SSE request framing changed source whitespace or parsed sentinel");
            int sent = transport.requests.size();
            fails("sse-id-state-must-be-explicit", () -> client.sendEvents(SendEventsInput.builder(List.of(Event.builder("one").id("one").build(), Event.builder("two").build())).build()));
            fails("unrepresentable-sse-data", () -> client.sendEvents(SendEventsInput.builder(List.of(Event.builder("a\rb").build())).build()));
            check(transport.requests.size() == sent, "unrepresentable SSE reached transport");
        }
    }
    private static void itemCeilings() {
        byte[] item = ("data: " + "x".repeat(1024 * 1024) + "\n\n").getBytes(StandardCharsets.UTF_8);
        byte[][] chunks = new byte[(item.length + 4095) / 4096][];
        for (int i = 0; i < chunks.length; i++) chunks[i] = Arrays.copyOfRange(item, i * 4096, Math.min(item.length, (i + 1) * 4096));
        var script = new NativeSupport.Script(200, chunks); script.headers.put("Content-Type", List.of("text/event-stream"));
        try (var client = new Client(options(new NativeSupport.Mock(script)).maxCaptureBytes(9).build()); var response = client.events()) {
            var failure = fails("resource-limit", () -> response.data().iterator().hasNext());
            check(failure.capture().length == 9 && failure.truncated(), "incremental item ceiling did not bound capture");
        }
        var bad = new NativeSupport.Mock(new NativeSupport.Script(200, "application/jsonl", "null\n"));
        try (var client = new Client(options(bad).codecLimits(ModelCodec.Limits.defaults().withEvaluationSteps(0)).build()); var response = client.nullableLines()) {
            fails("resource-limit", () -> response.data().nextAsync().join());
        }
    }
    public static void main(String[] args) throws Exception {
        zeroDemand(); throwingSubscriber(); finiteRequests(); itemCeilings();
        System.out.println("JAVA_STREAM_LIFETIME_OK: zero-demand terminals, cancellation, finite SSE requests and per-item work/byte ceilings");
    }
}
