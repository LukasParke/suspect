package {package};

import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.*;
import static {package}.JsonRuntime.*;

/** Single-consumer, context-owned parsed SSE envelopes or JSON-line values.
 * Closing either this stream or its response cancels the HTTP subscription.
 * Iterator, async pull and Flow consumption are mutually exclusive.
 * @param <T> source-bound item model
 */
public final class EventStream<T> implements Iterable<T>, AutoCloseable, Flow.Publisher<EventStream.Item<T>> {
    /** A non-null Flow item wrapper whose source value may represent JSON null.
     * @param value checked source value
     * @param <T> native item type
     */
    public record Item<T>(T value) {}

    private final InputStream input;
    private final ModelCodec<T> codec;
    private final ModelCodec.Limits limits;
    private final String source, framing;
    private final int status, itemLimit, captureLimit;
    private final Runnable release;
    private final Executor executor;
    private final AtomicBoolean closed = new AtomicBoolean(), pulling = new AtomicBoolean();
    private final AtomicReference<Thread> active = new AtomicReference<>();
    private final AtomicReference<SdkException> terminal = new AtomicReference<>();
    private volatile Runnable signal = () -> {};
    private String mode;
    private boolean eof, skipLF, first = true, hasData;
    private final ByteArrayOutputStream line = new ByteArrayOutputStream(), capture = new ByteArrayOutputStream();
    private final StringBuilder data = new StringBuilder();
    private String event, lastId;
    private byte[] lastCapture = new byte[0];
    private int lastItemBytes, itemBytes;
    private JsonNumber retry;

    EventStream(InputStream input, ModelCodec<T> codec, ModelCodec.Limits limits, String framing,
            int itemLimit, int captureLimit, String source, int status, Runnable release, Executor executor) {
        this.input = input; this.codec = codec; this.limits = limits; this.framing = framing;
        this.itemLimit = itemLimit; this.captureLimit = captureLimit; this.source = source; this.status = status;
        this.release = release; this.executor = executor;
    }

    private synchronized void claim(String requested) {
        if (mode == null) mode = requested;
        else if (!mode.equals(requested)) throw new IllegalStateException("stream already has a consumer");
    }
    private void checkTerminal() { SdkException error = terminal.get(); if (error != null) throw error; }
    private SdkException failure(String kind) { return new SdkException(kind, source, status, capture.toByteArray(), itemBytes > captureLimit); }
    private void rememberItem() { lastCapture = capture.toByteArray(); lastItemBytes = itemBytes; }
    private void resetItem() { itemBytes = 0; capture.reset(); }

    private JsonValue nextJson(ModelCodec.Context context) throws IOException {
        if (closed.get() || eof) return null;
        for (;;) {
            checkTerminal();
            int value = input.read();
            checkTerminal();
            if (value < 0) {
                eof = true;
                if (framing.equals("json-lines") && line.size() > 0) {
                    rememberItem();
                    return JsonRuntime.parse(line.toByteArray(), context.jsonBudget);
                }
                close(); return null;
            }
            context.spend(1);
            if (skipLF) { skipLF = false; if (value == '\n') continue; }
            if (++itemBytes > itemLimit) throw failure("resource-limit");
            if (capture.size() < captureLimit) capture.write(value);
            if (framing.equals("json-lines")) {
                if (value == '\n') {
                    rememberItem();
                    JsonValue result = JsonRuntime.parse(line.toByteArray(), context.jsonBudget);
                    line.reset(); resetItem(); return result;
                }
                line.write(value); continue;
            }
            if (value == '\r' || value == '\n') {
                skipLF = value == '\r';
                JsonValue result = finishSseLine(context);
                if (result != null) return result;
            } else line.write(value);
        }
    }

    private JsonValue finishSseLine(ModelCodec.Context context) {
        byte[] bytes = line.toByteArray(); line.reset(); int start = 0;
        if (first) {
            first = false;
            if (bytes.length >= 3 && (bytes[0] & 255) == 239 && (bytes[1] & 255) == 187 && (bytes[2] & 255) == 191) start = 3;
        }
        // HTML event-stream decoding replaces malformed UTF-8. JSON lines use
        // the strict exact JSON byte parser, sharing the item codec's budget.
        String text = context.string(new String(bytes, start, bytes.length - start, StandardCharsets.UTF_8));
        if (text.isEmpty()) {
            JsonValue result = null;
            if (hasData) {
                var fields = new LinkedHashMap<String, JsonValue>();
                fields.put("data", new JsonString(data.substring(0, data.length() - 1)));
                if (event != null && !event.isEmpty()) fields.put("event", new JsonString(event));
                if (lastId != null) fields.put("id", new JsonString(lastId));
                if (retry != null) fields.put("retry", retry);
                result = new JsonObject(fields);
            }
            rememberItem(); data.setLength(0); event = null; retry = null; hasData = false; resetItem();
            return result;
        }
        if (text.startsWith(":")) return null;
        int colon = text.indexOf(':');
        String name = colon < 0 ? text : text.substring(0, colon), value = colon < 0 ? "" : text.substring(colon + 1);
        if (value.startsWith(" ")) value = value.substring(1);
        switch (name) {
            case "data" -> { data.append(value).append('\n'); hasData = true; }
            case "event" -> event = value;
            case "id" -> { if (value.indexOf(0) < 0) lastId = value; }
            case "retry" -> {
                if (!value.isEmpty() && value.chars().allMatch(c -> c >= '0' && c <= '9')) {
                    int i = 0; while (i < value.length() && value.charAt(i) == '0') i++;
                    retry = JsonNumber.parse(i == value.length() ? "0" : value.substring(i));
                }
            }
            default -> { }
        }
        return null;
    }

    private Presence<T> pull() {
        Thread reader = Thread.currentThread(); active.set(reader);
        boolean parsed = false;
        try {
            checkTerminal();
            ModelCodec.Context context = new ModelCodec.Context(limits);
            JsonValue value = nextJson(context);
            if (value == null) return Presence.absent();
            parsed = true; checkTerminal();
            T result = codec.decodeValue(value, context);
            checkTerminal();
            // A final unterminated JSON line is still inside the deadline while
            // it is validated and converted. Only then release the exchange.
            if (eof) close();
            return Presence.of(result);
        } catch (SdkException error) {
            abort(error); throw error;
        } catch (CodecException error) {
            checkTerminal();
            String kind = switch (error.kind()) {
                case "resource", "evaluation_failure" -> "resource-limit";
                case "cancelled" -> "cancelled";
                default -> "invalid-stream-item";
            };
            SdkException failure = new SdkException(kind, source, status, parsed ? lastCapture : capture.toByteArray(),
                    (parsed ? lastItemBytes : itemBytes) > captureLimit, error.source(), error.instancePath());
            abort(failure); throw failure;
        } catch (JsonError error) {
            checkTerminal();
            SdkException failure = failure(error.kind().equals("resource") ? "resource-limit" : error.kind().equals("cancelled") ? "cancelled" : "invalid-stream-item");
            abort(failure); throw failure;
        } catch (IOException error) {
            checkTerminal();
            if (closed.get()) return Presence.absent();
            SdkException failure = failure(reader.isInterrupted() ? "cancelled" : "transport");
            abort(failure); throw failure;
        } finally { active.compareAndSet(reader, null); }
    }

    /** Obtain the sole blocking iterator. Use try-with-resources for early exit. @return iterator */
    @Override public synchronized Iterator<T> iterator() {
        if (mode != null) throw new IllegalStateException("stream already has a consumer"); mode = "iterator";
        return new Iterator<>() {
            private T next; private boolean ready, done;
            @Override public boolean hasNext() {
                checkTerminal();
                if (!ready && !done) { Presence<T> item = pull(); done = !item.isPresent(); if (!done) next = item.value(); ready = !done; }
                return !done;
            }
            @Override public T next() {
                if (!hasNext()) throw new NoSuchElementException();
                T value = next; next = null; ready = false; return value;
            }
        };
    }

    /** Pull one item asynchronously, without buffering later items.
     * @return empty on EOF; cancellation closes the response subscription
     */
    public CompletableFuture<Presence<T>> nextAsync() {
        claim("async");
        if (!pulling.compareAndSet(false, true)) throw new IllegalStateException("an async pull is already in progress");
        if (terminal.get() != null) { pulling.set(false); return CompletableFuture.failedFuture(terminal.get()); }
        if (closed.get() || eof) { pulling.set(false); return CompletableFuture.completedFuture(Presence.absent()); }
        CompletableFuture<Presence<T>> result = new CompletableFuture<>();
        result.whenComplete((value, error) -> { if (result.isCancelled()) close(); });
        try {
            executor.execute(() -> {
                try { Presence<T> value = pull(); pulling.set(false); result.complete(value); }
                catch (RuntimeException error) { pulling.set(false); result.completeExceptionally(error); }
            });
        } catch (RejectedExecutionException error) {
            pulling.set(false); SdkException failure = failure("closed"); abort(failure); result.completeExceptionally(failure);
        }
        return result;
    }

    /** Subscribe with Flow backpressure. Only requested items are decoded.
     * @param subscriber sole subscriber
     */
    @Override public void subscribe(Flow.Subscriber<? super Item<T>> subscriber) {
        Objects.requireNonNull(subscriber);
        synchronized (this) {
            if (mode != null) {
                subscriber.onSubscribe(new Flow.Subscription() { public void request(long n) {} public void cancel() {} });
                subscriber.onError(new IllegalStateException("stream already has a consumer")); return;
            }
            mode = "flow";
        }
        Publisher subscription = new Publisher(subscriber);
        // Install the notification after onSubscribe: no terminal callback may
        // precede it, including a deadline that fires while subscribing.
        try { subscriber.onSubscribe(subscription); }
        catch (RuntimeException error) { subscription.cancel(); return; }
        signal = subscription::start; subscription.subscribed = true;
        if (subscription.demand.get() > 0 || closed.get() || terminal.get() != null) subscription.start();
    }

    private final class Publisher implements Flow.Subscription, Runnable {
        private final Flow.Subscriber<? super Item<T>> subscriber;
        private final AtomicLong demand = new AtomicLong();
        private final AtomicBoolean running = new AtomicBoolean(), cancelled = new AtomicBoolean(), terminated = new AtomicBoolean();
        private final AtomicReference<Throwable> invalidDemand = new AtomicReference<>();
        private volatile boolean subscribed;
        Publisher(Flow.Subscriber<? super Item<T>> subscriber) { this.subscriber = subscriber; }
        @Override public void request(long n) {
            if (cancelled.get() || terminated.get()) return;
            if (n <= 0) { invalidDemand.compareAndSet(null, new IllegalArgumentException("Flow demand must be positive")); close(); start(); return; }
            demand.updateAndGet(old -> old > Long.MAX_VALUE - n ? Long.MAX_VALUE : old + n); start();
        }
        void start() {
            if (!subscribed || cancelled.get() || terminated.get() || !running.compareAndSet(false, true)) return;
            try { executor.execute(this); }
            catch (RejectedExecutionException error) {
                // No other drain is active. Serialize the terminal notification
                // here if the owning client has already stopped its executor.
                invalidDemand.compareAndSet(null, terminal.get() == null ? failure("closed") : terminal.get());
                run();
            }
        }
        private Throwable error() { return invalidDemand.get() == null ? terminal.get() : invalidDemand.get(); }
        private void end(Throwable error) {
            if (cancelled.get() || !terminated.compareAndSet(false, true)) return;
            try { if (error == null) subscriber.onComplete(); else subscriber.onError(error); }
            catch (RuntimeException ignored) { }
        }
        @Override public void run() {
            try {
                while (!cancelled.get() && !terminated.get()) {
                    Throwable failure = error();
                    if (failure != null) { end(failure); return; }
                    if (closed.get() || eof) { end(null); return; }
                    if (demand.get() == 0) return;
                    Presence<T> value;
                    try { value = pull(); } catch (RuntimeException error) { end(error); return; }
                    if (error() != null) { end(error()); return; }
                    if (!value.isPresent()) { end(null); return; }
                    if (demand.get() != Long.MAX_VALUE) demand.decrementAndGet();
                    if (!cancelled.get()) {
                        try { subscriber.onNext(new Item<>(value.value())); }
                        catch (RuntimeException error) { cancel(); return; }
                    }
                }
            } finally {
                running.set(false);
                if (!cancelled.get() && !terminated.get() && (demand.get() > 0 || error() != null || closed.get() || eof)) start();
            }
        }
        @Override public void cancel() { if (cancelled.compareAndSet(false, true)) close(); }
    }

    void abort(SdkException error) {
        synchronized (closed) { if (closed.get()) return; terminal.compareAndSet(null, error); }
        close();
    }
    /** Close early, release buffered bytes and cancel owned transport work. */
    @Override public void close() {
        synchronized (closed) { if (!closed.compareAndSet(false, true)) return; }
        try { input.close(); } catch (IOException | RuntimeException ignored) { }
        finally {
            Thread reader = active.get();
            if (reader != null && reader != Thread.currentThread()) reader.interrupt();
            release.run(); signal.run();
        }
    }
}
