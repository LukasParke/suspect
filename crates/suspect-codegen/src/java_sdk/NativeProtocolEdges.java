import example.protocol.*;
import example.protocol.Client.*;
import static example.protocol.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;

/** Additional protocol-specific boundary tests, independent of shared serialization. */
public final class NativeProtocolEdges {
    private NativeProtocolEdges() {}
    private static final List<String> failures = new ArrayList<>();
    private static int cases;
    @FunctionalInterface interface Check { void run() throws Exception; }
    private static void test(String name, Check check) {
        cases++;
        try { check.run(); }
        catch (Exception | AssertionError error) { failures.add(name + ": " + error); error.printStackTrace(); }
    }
    private static void check(boolean condition, String message) { if (!condition) throw new AssertionError(message); }
    private static SdkException fails(String kind, Runnable action) { return NativeProtocolControls.fails(kind, action); }
    private static NativeSupport.Script json(String text) { return new NativeSupport.Script(200, "application/json", text); }
    private static HttpRuntime.Options.Builder options(NativeSupport.Mock transport) { return HttpRuntime.Options.builder().httpClient(transport); }
    private static NativeSupport.Mock stream(String media, byte[][] chunks) {
        var script = new NativeSupport.Script(200, chunks);
        script.headers.put("Content-Type", List.of(media));
        return new NativeSupport.Mock(script);
    }
    private static byte[] bytes(String text) { return text.getBytes(StandardCharsets.UTF_8); }

    private static void streamChunks() {
        for (String media : List.of("text/event-stream", "application/jsonl")) {
            byte[] wire = bytes(media.equals("text/event-stream") ? "\uFEFFid: café\r\ndata: 雪😀\r\n\r\ndata: tail\n\n" : "\"雪😀\"\r\nnull\n\"tail\"");
            for (int split = 0; split <= wire.length; split++) {
                var transport = stream(media, new byte[][] {Arrays.copyOfRange(wire, 0, split), Arrays.copyOfRange(wire, split, wire.length)});
                try (var client = new Client(options(transport).build())) {
                    if (media.equals("text/event-stream")) {
                        try (var response = client.events()) {
                            var values = new ArrayList<Event>(); response.data().forEach(values::add);
                            check(values.size() == 2 && values.getFirst().data().equals("雪😀") && values.getLast().id().value().equals("café"), "SSE chunk boundary " + split);
                        }
                    } else {
                        try (var response = client.nullableLines()) {
                            var values = new ArrayList<String>(); response.data().forEach(values::add);
                            check(values.equals(Arrays.asList("雪😀", null, "tail")), "JSON-lines chunk boundary " + split);
                        }
                    }
                }
            }
        }
    }

    private static void serializedFlowTerminal() throws Exception {
        var transport = stream("text/event-stream", new byte[][] {bytes("data: first\n\ndata: second\n\n")});
        try (var client = new Client(options(transport).build()); var response = client.events()) {
            var entered = new CountDownLatch(1); var leave = new CountDownLatch(1); var done = new CompletableFuture<Void>();
            var subscription = new AtomicReference<Flow.Subscription>(); var inside = new AtomicBoolean(); var overlap = new AtomicBoolean(); var terminals = new AtomicInteger();
            response.data().subscribe(new Flow.Subscriber<EventStream.Item<Event>>() {
                public void onSubscribe(Flow.Subscription value) { subscription.set(value); value.request(1); }
                public void onNext(EventStream.Item<Event> item) {
                    inside.set(true); entered.countDown();
                    try { leave.await(3, TimeUnit.SECONDS); } catch (InterruptedException error) { Thread.currentThread().interrupt(); }
                    finally { inside.set(false); }
                }
                public void onError(Throwable error) { overlap.set(inside.get()); terminals.incrementAndGet(); done.complete(null); }
                public void onComplete() { terminals.incrementAndGet(); done.completeExceptionally(new AssertionError("invalid demand completed normally")); }
            });
            check(entered.await(3, TimeUnit.SECONDS), "no first Flow callback");
            subscription.get().request(0); leave.countDown(); done.get(3, TimeUnit.SECONDS);
            check(!overlap.get() && terminals.get() == 1, "Flow onError overlapped onNext or repeated");
        }
    }

    private static void textHeaders() {
        var transport = new NativeSupport.Mock(json("{\"ok\":true}"));
        transport.script.headers.put("X-Count", List.of("1"));
        transport.script.headers.put("X-Text-Int", List.of("20e-1"));
        transport.script.headers.put("X-Text-Bool", List.of("false"));
        try (var client = new Client(options(transport).build())) {
            var headers = client.readMetadata().typedHeaders();
            check(headers.xTextInt().value().exactIntegerValue().intValueExact() == 2 && !headers.xTextBool().value(), "typed non-JSON header content");
        }
    }

    private static void serverChoices() {
        var transport = new NativeSupport.Mock(json("{\"ok\":true}"));
        try (var client = new Client(options(transport).documentUrl(URI.create("https://docs.example.test/specs/openapi.json")).serverName("regional").build())) {
            client.serverChoice(RequestOptions.builder().serverIndex(0).build());
            check(transport.requests.getLast().uri().equals("https://docs.example.test/v1/server"), "per-call index did not override default name");
        }
        try (var client = new Client(options(transport).serverUrl(URI.create("https://münich.example.test/base")).build())) {
            client.unicodePath();
            check(transport.requests.getLast().uri().equals("https://xn--mnich-kva.example.test/base/snow/%E9%9B%AA/%F0%9F%98%80"), "Unicode host/path URI");
        }
    }

    private static void bufferedDeadline() {
        var script = json("{\"ok\":"); script.stallBody = true;
        var transport = new NativeSupport.Mock(script);
        try (var client = new Client(options(transport).timeout(Duration.ofMillis(150)).maxCaptureBytes(4).build())) {
            var error = fails("timeout", client::publicValue);
            check(error.status() == 200 && Arrays.equals(error.capture(), bytes("{\"ok")) && error.truncated(), "partial buffered timeout lost status/capture");
        }
        var before = json("{}"); before.stallHeaders = true;
        try (var client = new Client(options(new NativeSupport.Mock(before)).timeout(Duration.ofMillis(150)).build())) { fails("timeout", client::publicValue); }
    }

    private static void parsingLimits() {
        var transport = stream("application/jsonl", new byte[][] {bytes("\"123456789\"\n")});
        try (var client = new Client(options(transport).codecLimits(ModelCodec.Limits.defaults().withJson(JsonRuntime.Limits.defaults().withInputBytes(4))).build()); var response = client.nullableLines()) {
            fails("resource-limit", () -> response.data().iterator().hasNext());
        }
        var invalid = stream("application/x-ndjson", new byte[][] {bytes("{\"n\":\"bad\"}")});
        try (var client = new Client(options(invalid).maxCaptureBytes(6).build()); var response = client.lines()) {
            var error = fails("invalid-stream-item", () -> response.data().nextAsync().join());
            check(error.capture().length == 6 && error.truncated() && error.instancePath().equals("/n"), "final unterminated item capture/source");
        }
        for (String wire : List.of("\n", "{} {}\n", "{\"n\":1}\n\n")) {
            var blank = stream("application/x-ndjson", new byte[][] {bytes(wire)});
            try (var client = new Client(options(blank).build()); var response = client.lines()) {
                fails("invalid-stream-item", () -> response.data().forEach(value -> {}));
            }
        }
    }

    private static void typedErrorStream() throws Exception {
        var script = new NativeSupport.Script(400, "text/event-stream", "data: error detail\n\n"); script.stallBody = true;
        var transport = new NativeSupport.Mock(script);
        try (var client = new Client(options(transport).timeout(Duration.ofMillis(200)).build())) {
            try { client.errorEvents(); throw new AssertionError("declared stream error became success"); }
            catch (ErrorEventsStatus400 error) {
                try (error) {
                    check(error.data().nextAsync().get().value().data().equals("error detail"), "typed API error lost stream ownership");
                    check(fails("timeout", () -> error.data().nextAsync().join()).status() == 400, "error stream deadline status");
                }
            }
        }
    }

    private static void nativeStructure() {
        var title = UploadMultipartTitlePart.builder("title").build();
        var file = UploadMultipartFilePart.builder(Bytes.of(new byte[] {0, (byte) 255}), UploadMultipartFileHeaders.builder(JsonNumber.of(2)).build()).build();
        var tags = new ArrayList<UploadMultipartTagsPart>(); tags.add(UploadMultipartTagsPart.builder("one").build());
        var builder = UploadMultipart.builder(file, title).tags(tags); var snapshot = builder.build();
        tags.clear(); builder.omitTags();
        check(snapshot.tags().value().size() == 1 && snapshot.file().value().size() == 2, "multipart builder did not deep snapshot");
        fails("wire-cardinality", () -> UploadMultipart.builder(file, title).tags(List.of()).build());
        fails("wire-cardinality", () -> UploadMultipart.builder(file, title).tags(Collections.nCopies(3, UploadMultipartTagsPart.builder("many").build())).build());
        fails("resource-limit", () -> UploadMultipartFilePart.builder(Bytes.of(new byte[33]), UploadMultipartFileHeaders.builder(JsonNumber.of(33)).build()).build());
        try { SendPositionalMultipart.builder(SendPositionalMultipartPart1Part.builder("prefix").build()).addItem(SendPositionalMultipartAdditionalPart.builder(Line.builder(JsonNumber.of(1)).build()).build()).build(); throw new AssertionError("positional hole was accepted"); }
        catch (CodecException expected) { check(expected.kind().equals("invalid"), "positional hole kind"); }
    }

    public static void main(String[] args) throws Exception {
        test("arbitrary stream chunks", NativeProtocolEdges::streamChunks);
        test("serialized Flow terminal", NativeProtocolEdges::serializedFlowTerminal);
        test("typed text headers", NativeProtocolEdges::textHeaders);
        test("source server precedence and Unicode", NativeProtocolEdges::serverChoices);
        test("buffered whole-call deadlines", NativeProtocolEdges::bufferedDeadline);
        test("stream parser budgets and EOF", NativeProtocolEdges::parsingLimits);
        test("typed streaming API errors", NativeProtocolEdges::typedErrorStream);
        test("native aggregate structure", NativeProtocolEdges::nativeStructure);
        if (!failures.isEmpty()) throw new AssertionError(String.join("\n", failures));
        System.out.println("JAVA_PROTOCOL_EDGES_OK: " + cases + " protocol boundary groups");
    }
}
