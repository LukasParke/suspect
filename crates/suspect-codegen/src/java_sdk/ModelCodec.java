package {package};

import java.util.*;
import java.util.function.*;
import static {package}.JsonRuntime.*;

/** Typed source-bound codec. A complete call shares parser/writer, conversion,
 * equality and schema-evaluation budgets across every nested value and arm.
 * No nested conversion, generic JSON value or union trial creates a fresh budget.
 *
 * @param <T> native source-selected type
 */
public final class ModelCodec<T> {
    /** Immutable per-call policy. Validation allowances can only narrow the
     * limits embedded in the checked program.
     * @param json parser/writer byte, depth and work limits
     * @param maxConversionDepth maximum nested native conversions
     * @param maxConversionSteps visits and processed/copied UTF-8 bytes, shared with validation byte work
     * @param maxEvaluationSteps schema/keyword/collection visits
     * @param maxEqualitySteps structural equality node pairs
     */
    public record Limits(JsonRuntime.Limits json, int maxConversionDepth, long maxConversionSteps, long maxEvaluationSteps, long maxEqualitySteps) {
        /** Validate the finite policy. */
        public Limits {
            Objects.requireNonNull(json);
            if (maxConversionDepth < 0 || maxConversionDepth > MAX_DEPTH || maxConversionSteps < 0 || maxConversionSteps > 256L * MAX_BYTES
                    || maxEvaluationSteps < 0 || maxEvaluationSteps > 100_000_000 || maxEqualitySteps < 0 || maxEqualitySteps > 100_000_000)
                throw new IllegalArgumentException("invalid codec resource limits");
        }
        /** Default finite codec policy. @return immutable policy */
        public static Limits defaults() { return new Limits(JsonRuntime.Limits.defaults(), MAX_DEPTH, 32L * 1024 * 1024, 100_000, 100_000); }
        /** Set parser/writer limits. @param value policy @return new policy */
        public Limits withJson(JsonRuntime.Limits value) { return new Limits(value, maxConversionDepth, maxConversionSteps, maxEvaluationSteps, maxEqualitySteps); }
        /** Set conversion/byte work. @param value units @return new policy */
        public Limits withConversionSteps(long value) { return new Limits(json, maxConversionDepth, value, maxEvaluationSteps, maxEqualitySteps); }
        /** Set conversion nesting. @param value depth @return new policy */
        public Limits withConversionDepth(int value) { return new Limits(json, value, maxConversionSteps, maxEvaluationSteps, maxEqualitySteps); }
        /** Set schema visits. @param value visits @return new policy */
        public Limits withEvaluationSteps(long value) { return new Limits(json, maxConversionDepth, maxConversionSteps, value, maxEqualitySteps); }
        /** Set equality comparisons. @param value pairs @return new policy */
        public Limits withEqualitySteps(long value) { return new Limits(json, maxConversionDepth, maxConversionSteps, maxEvaluationSteps, value); }
    }

    private final int root;
    private final BiFunction<JsonValue, Context, T> reader;
    private final BiFunction<T, Context, JsonValue> writer;
    private final Limits limits;
    ModelCodec(int root, BiFunction<JsonValue, Context, T> reader, BiFunction<T, Context, JsonValue> writer) {
        this(root, reader, writer, Limits.defaults());
    }
    private ModelCodec(int root, BiFunction<JsonValue, Context, T> reader, BiFunction<T, Context, JsonValue> writer, Limits limits) {
        this.root = root; this.reader = reader; this.writer = writer; this.limits = Objects.requireNonNull(limits);
    }
    /** Original source binding. @return absolute document plus escaped pointer */
    public String source() { return Validation.bundled().sourceAt(root); }
    /** Per-call policy for this codec. @return immutable limits */
    public Limits limits() { return limits; }
    /** Create a codec with an explicit finite policy.
     * @param limits policy
     * @return immutable codec retaining the exact same source/type binding
     */
    public ModelCodec<T> withLimits(Limits limits) { return new ModelCodec<>(root, reader, writer, limits); }
    /** Decode one complete JSON document.
     * @param json document
     * @return validated immutable native value
     * @throws CodecException for invalid JSON, schema mismatch, resource exhaustion or cancellation
     */
    public T decode(String json) {
        Context context = new Context(limits);
        return context.bound(root, () -> readChecked(JsonRuntime.parse(json, context.jsonBudget), context));
    }
    /** Decode strict UTF-8 JSON.
     * @param json document bytes
     * @return validated immutable native value
     * @throws CodecException for invalid JSON, schema mismatch, resource exhaustion or cancellation
     */
    public T decode(byte[] json) { return decode(json, new Context(limits)); }
    T decode(byte[] json, Context context) { return context.bound(root, () -> readChecked(JsonRuntime.parse(json, context.jsonBudget), context)); }
    /** Encode exact JSON, revalidating every value and selected arm.
     * @param value native value
     * @return complete exact JSON document
     * @throws CodecException for invalid representation, schema mismatch or incomplete evaluation
     */
    public String encode(T value) {
        Context context = new Context(limits);
        return context.bound(root, () -> JsonRuntime.stringify(writeChecked(value, context), context.jsonBudget));
    }
    /** Decode an immutable JSON value under the same byte/depth policy.
     * @param value JSON value
     * @return validated immutable native value
     */
    public T decodeValue(JsonValue value) {
        Context context = new Context(limits);
        return context.bound(root, () -> {
            context.json(value);
            JsonRuntime.stringify(value, context.jsonBudget);
            return readChecked(value, context);
        });
    }
    T decodeValue(JsonValue value, Context context) { return context.bound(root, () -> readChecked(value, context)); }
    /** Encode to the immutable JSON domain, with checked wire-size admission.
     * @param value native value
     * @return exact JSON value
     */
    public JsonValue encodeValue(T value) {
        Context context = new Context(limits);
        return context.bound(root, () -> {
            JsonValue json = writeChecked(value, context);
            JsonRuntime.stringify(json, context.jsonBudget);
            return json;
        });
    }
    JsonValue encodeValue(T value, Context context) { return context.bound(root, () -> writeChecked(value, context)); }
    /** Validate and deeply snapshot construction state in one shared session.
     * @param value native value, possibly containing caller-owned lists
     * @return independent immutable native value
     */
    public T snapshot(T value) { return snapshot(value, new Context(limits)); }
    T snapshot(T value, Context context) {
        return context.bound(root, () -> {
            JsonValue json = writeChecked(value, context);
            JsonRuntime.stringify(json, context.jsonBudget);
            return reader.apply(json, context);
        });
    }
    private T readChecked(JsonValue json, Context context) { context.check(root, json); return reader.apply(json, context); }
    private JsonValue writeChecked(T value, Context context) {
        JsonValue json = writer.apply(value, context); context.check(root, json); return json;
    }
    // Package-private conversion seam used only by generated nested bindings.
    // Public entry points above always create/check a complete bounded call.
    T read(JsonValue value, Context context) { return reader.apply(value, context); }
    JsonValue write(T value, Context context) { return writer.apply(value, context); }

    static final class Context {
        private record Frame(int previousRoot, int key, Object value) {}
        final JsonRuntime.Budget jsonBudget;
        private final Limits limits;
        private final Validation.Program program;
        private final Validation.Session validation;
        private final ArrayDeque<Frame> stack = new ArrayDeque<>();
        private final IdentityHashMap<Object, Set<Integer>> active = new IdentityHashMap<>();
        private long steps;
        private int currentRoot = -1;
        private String path = "";
        Context() { this(Limits.defaults()); }
        Context(Limits limits) {
            this.limits = limits; steps = limits.maxConversionSteps(); jsonBudget = new JsonRuntime.Budget(limits.json());
            program = Validation.bundled();
            validation = program.session(limits.maxEvaluationSteps(), limits.maxEqualitySteps(), this::spend);
        }
        String source() { return program.sourceAt(currentRoot); }
        String path() { return path; }
        CodecException invalid(String message) { return new CodecException("invalid", source(), path, message); }
        void spend(long amount) {
            if (Thread.currentThread().isInterrupted()) throw new CodecException("cancelled", source(), path, "interrupted");
            if (amount < 0 || steps < amount) throw new CodecException("resource", source(), path, "shared conversion/byte-work ceiling");
            steps -= amount;
        }
        void enter(int root, Object value) {
            int previous = currentRoot; if (root >= 0) currentRoot = root;
            spend(1);
            if (stack.size() >= limits.maxConversionDepth()) throw new CodecException("resource", source(), path, "native conversion depth ceiling");
            if (value != null && !active.computeIfAbsent(value, ignored -> new HashSet<>()).add(root)) throw invalid("cyclic native representation");
            stack.push(new Frame(previous, root, value));
        }
        void leave() {
            Frame frame = stack.pop(); currentRoot = frame.previousRoot();
            if (frame.value() != null) {
                Set<Integer> roots = active.get(frame.value()); roots.remove(frame.key());
                if (roots.isEmpty()) active.remove(frame.value());
            }
        }
        <V> V bound(int root, Supplier<V> action) {
            int previous = currentRoot; currentRoot = root;
            try { return guarded(action); } finally { currentRoot = previous; }
        }
        <V> V at(String name, Supplier<V> action) {
            String previous = path; path = Validation.child(path, name);
            try { return guarded(action); } finally { path = previous; }
        }
        private <V> V guarded(Supplier<V> action) {
            try { return action.get(); }
            catch (JsonError error) { throw new CodecException(error.kind(), source(), path, "JSON syntax or resource policy rejected value"); }
            catch (ClassCastException | NullPointerException error) { throw invalid("native value does not match the model"); }
        }
        void check(int root, JsonValue value) { validation.check(root, value, path); }
        boolean matches(int root, JsonValue value) { return validation.matches(root, value, path); }
        <V> V require(V value) { if (value == null) throw invalid("Java null is not this native value"); spend(1); return value; }
        String string(String value) {
            if (value == null) throw invalid("missing non-null string");
            spend(value.length());
            int bytes = JsonRuntime.utf8Length(value); spend(bytes - value.length());
            return value;
        }
        JsonNumber number(JsonNumber value) {
            if (value == null) throw invalid("missing exact number");
            if (value.token().length() > limits.json().maxNumberBytes()) throw new CodecException("resource", source(), path, "numeric token byte ceiling");
            spend(value.token().length()); return value;
        }
        JsonNull nullValue(Object value) { if (value != JsonNull.INSTANCE) throw invalid("expected JsonNull.INSTANCE"); spend(1); return JsonNull.INSTANCE; }
        <V> V never() { throw invalid("source false schema has no value"); }
        JsonValue json(JsonValue value) {
            enter(-1, require(value));
            try {
                switch (value) {
                    case JsonString text -> string(text.value());
                    case JsonNumber number -> number(number);
                    case JsonArray array -> {
                        for (int i = 0; i < array.values().size(); i++) {
                            JsonValue item = array.values().get(i); at(Integer.toString(i), () -> json(item));
                        }
                    }
                    case JsonObject object -> {
                        for (var entry : object.values().entrySet()) { String key = string(entry.getKey()); at(key, () -> json(entry.getValue())); }
                    }
                    case JsonNull ignored -> { }
                    case JsonBoolean ignored -> { }
                }
                // All six JsonValue implementations are sealed and deeply
                // immutable; a metered traversal can safely retain the value.
                return value;
            } finally { leave(); }
        }
        <V> List<V> readList(JsonValue value, Function<JsonValue, V> read) {
            enter(-1, value);
            try {
                var result = new ArrayList<V>(); var items = ((JsonArray) value).values();
                for (int i = 0; i < items.size(); i++) { spend(1); JsonValue item = items.get(i); result.add(at(Integer.toString(i), () -> read.apply(item))); }
                return Collections.unmodifiableList(result);
            } finally { leave(); }
        }
        <V> JsonValue writeList(List<V> value, Function<V, JsonValue> write) {
            enter(-1, require(value));
            try {
                var result = new ArrayList<JsonValue>(); int index = 0;
                for (V item : value) { spend(1); result.add(at(Integer.toString(index++), () -> write.apply(item))); }
                return new JsonArray(result);
            } finally { leave(); }
        }
    }
}
