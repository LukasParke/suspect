package {package};

import java.io.IOException;
import java.net.URI;
import java.util.*;
import java.util.function.LongConsumer;
import static {package}.JsonRuntime.*;

/** Executes checked OwnedCompiler instructions, never an OpenAPI/schema interpreter.
 * Invalidity, evaluation exhaustion and nonproductive recursion remain distinct.
 */
public final class Validation {
    private Validation() {}
    private static final class Bundled {
        private static final Program PROGRAM = load();
        private static Program load() {
            try (var stream = Validation.class.getResourceAsStream("validation-program.json")) {
                if (stream == null) throw malformed("", "missing checked validation program");
                return Program.fromJson(stream.readNBytes(MAX_BYTES + 1));
            } catch (IOException error) { throw malformed("", "cannot load checked validation program"); }
        }
    }
    /** The immutable source-bound program installed with this jar. @return checked program */
    public static Program bundled() { return Bundled.PROGRAM; }

    /** Immutable finite validation graph. Public loading checks the version,
     * opcodes, operands, source identities, graph edges and applicator invariants.
     * The generator also checks the original OwnedProgram before emission.
     */
    public static final class Program {
        private static final Set<String> SCOPED_OPS = Set.of("if", "dependentRequired", "dependentSchemas", "contains", "patternProperties", "additionalPropertiesWithPatterns", "propertyNames", "unevaluatedProperties", "unevaluatedItems");
        private final List<JsonValue> nodes;
        private final Map<String, Integer> roots;
        private final Set<Integer> rootIndices;
        private final int maxDepth, maxNumberBytes;
        private final long maxSteps, maxEqualities;
        private final IdentityHashMap<JsonObject, JsonNumber> operands = new IdentityHashMap<>();
        private final IdentityHashMap<JsonObject, Pattern> patterns = new IdentityHashMap<>();
        private final IdentityHashMap<JsonObject, Set<String>> declaredNames = new IdentityHashMap<>();
        private final IdentityHashMap<JsonObject, List<PatternRule>> patternGroups = new IdentityHashMap<>();
        private final IdentityHashMap<JsonObject, ContainsBounds> containsBounds = new IdentityHashMap<>();
        private final boolean scoped, dynamic;
        private final ValidationResources resources;
        private Program(JsonObject raw) {
            String version = str(raw, "version"), profile = str(raw, "profile");
            dynamic = version.equals("suspect.validation.experimental.v3") && profile.equals("oas31-jsonschema202012-resources-dynamic");
            scoped = dynamic || version.equals("suspect.validation.experimental.v2") && profile.equals("oas31-jsonschema202012-static-applicators");
            if (!scoped && !(version.equals("suspect.validation.experimental.v1") && profile.equals("oas31-jsonschema202012-static-subset")))
                throw malformed("", "unsupported validation version or profile");
            if (!dynamic && raw.values().containsKey("resourceContext")) throw malformed("", "resource metadata requires v3");
            if (scoped && !raw.values().keySet().equals(dynamic ? Set.of("version", "profile", "nodes", "roots", "limits", "resourceContext") : Set.of("version", "profile", "nodes", "roots", "limits"))) throw malformed("", "unknown scoped validation envelope members");
            nodes = arr(get(raw, "nodes"));
            if (nodes.size() > 100_000) throw malformed("", "validation graph allocation ceiling");
            JsonObject limits = obj(get(raw, "limits"));
            maxDepth = integer(limits, "maxDepth"); maxNumberBytes = integer(limits, "maxNumberBytes");
            maxSteps = integer(limits, "maxEvaluationSteps"); maxEqualities = integer(limits, "maxEqualitySteps");
            int maxErrors = integer(limits, "maxErrors");
            if (maxDepth < 0 || maxDepth > MAX_DEPTH || maxNumberBytes < 0 || maxNumberBytes > MAX_NUMBER_BYTES
                    || maxSteps < 0 || maxSteps > 100_000_000 || maxEqualities < 0 || maxEqualities > 100_000_000 || maxErrors < 0)
                throw malformed("", "unsupported validation resource limits");
            var identities = new HashSet<String>();
            for (JsonValue node : nodes) if (!identities.add(checkedSource(obj(node)))) throw malformed("", "duplicate schema identity");
            resources = dynamic ? new ValidationResources(obj(get(raw, "resourceContext")), nodes) : null;
            for (JsonValue node : nodes) checkNode(obj(node));
            var selected = new LinkedHashMap<String, Integer>();
            var indices = new HashSet<Integer>();
            for (JsonValue value : arr(get(raw, "roots"))) {
                JsonObject root = obj(value); String at = checkedSource(root); int target = target(root, "target", at);
                if (!source(obj(nodes.get(target))).equals(at) || selected.putIfAbsent(at, target) != null)
                    throw malformed(at, "inconsistent or duplicate selected root");
                indices.add(target);
            }
            roots = Collections.unmodifiableMap(selected); rootIndices = Set.copyOf(indices);
        }
        /** Load a portable program without weakening unknown instructions.
         * @param bytes complete OwnedProgram JSON bytes
         * @return immutable checked program
         * @throws CodecException if the program is malformed or unsupported
         */
        public static Program fromJson(byte[] bytes) {
            try { return new Program(obj(JsonRuntime.parse(bytes))); }
            catch (CodecException error) { throw error; }
            catch (IllegalArgumentException | ClassCastException | NullPointerException | ArithmeticException error) { throw malformed("", "malformed checked validation program"); }
        }
        /** Original document-plus-pointer identities and selected node indices.
         * @return immutable selected roots
         */
        public Map<String, Integer> roots() { return roots; }
        /** Look up one selected source. @param source absolute document plus escaped pointer @return root index */
        public int root(String source) {
            Integer result = roots.get(source);
            if (result == null) throw malformed(source, "source is not a selected root");
            return result;
        }
        /** Check a JSON value in a fresh bounded session.
         * @param root selected node index
         * @param value immutable JSON value
         * @throws CodecException on invalidity or incomplete evaluation
         */
        public void check(int root, JsonValue value) {
            long[] bytes = {32L * 1024 * 1024};
            session(maxSteps, maxEqualities, amount -> {
                if (amount < 0 || bytes[0] < amount) throw new CodecException("evaluation_failure", sourceAt(root), "", "validation byte-work ceiling");
                bytes[0] -= amount;
            }).check(root, value, "");
        }
        String sourceAt(int root) { return root >= 0 && root < nodes.size() ? source(obj(nodes.get(root))) : ""; }
        Session session(long steps, long equality, LongConsumer bytes) {
            return new Session(this, Math.min(steps, maxSteps), Math.min(equality, maxEqualities), bytes);
        }
        private int target(JsonObject check, String key, String at) {
            int index;
            try { index = integer(check, key); }
            catch (ArithmeticException | ClassCastException | NullPointerException error) { throw malformed(at, "target must be a finite integer index"); }
            if (index < 0 || index >= nodes.size()) throw malformed(at, "target is outside finite graph");
            return index;
        }
        private void located(int index, String expected, String at) {
            if (!source(obj(nodes.get(index))).equals(expected)) throw malformed(at, "applicator target has an inconsistent source");
        }
        private int optionalTarget(JsonObject check, String key, String at) { return get(check, key) == JsonNull.INSTANCE ? -1 : target(check, key, at); }
        private JsonNumber countOperand(JsonObject check, String key, String at) {
            if (get(check, key) == JsonNull.INSTANCE) return null;
            String value = str(check, key);
            if (value.length() > maxNumberBytes) throw malformed(at, "contains operand byte ceiling");
            JsonNumber number = JsonNumber.parse(value);
            if (!number.isInteger() || number.signum() < 0) throw malformed(at, "contains count must be a nonnegative mathematical integer");
            return number;
        }
        private void checkNode(JsonObject node) {
            String at = source(node);
            Set<String> locations = new HashSet<>(), properties = new HashSet<>(), declared = null;
            List<JsonValue> checks = arr(get(node, "checks"));
            int prefix = 0, start = 0; boolean hasItems = false;
            boolean additionalPatterns = false, unevaluated = false; String additionalAt = at;
            List<PatternRule> rules = List.of();
            for (JsonValue value : checks) {
                JsonObject check = obj(value); String location = checkedSource(check), op = str(check, "op");
                if (SCOPED_OPS.contains(op) && !scoped) throw malformed(location, "scoped opcode requires v2");
                if (op.equals("dynamicRef") && !dynamic) throw malformed(location, "dynamic opcode requires v3");
                if (op.equals("unevaluatedProperties") || op.equals("unevaluatedItems")) unevaluated = true;
                else if (unevaluated) throw malformed(location, "unevaluated instructions must be last");
                String keyword = switch (op) {
                    case "always" -> "";
                    case "ref" -> "$ref";
                    case "dynamicRef" -> "$dynamicRef";
                    case "additionalPropertiesWithPatterns" -> "additionalProperties";
                    case "bound" -> bool(check, "maximum") ? (bool(check, "exclusive") ? "exclusiveMaximum" : "maximum") : (bool(check, "exclusive") ? "exclusiveMinimum" : "minimum");
                    case "count" -> (bool(check, "maximum") ? "max" : "min") + switch (str(check, "target")) {
                        case "string" -> "Length"; case "array" -> "Items"; case "object" -> "Properties";
                        default -> throw malformed(location, "unknown count target");
                    };
                    case "type", "properties", "additionalProperties", "required", "items", "prefixItems", "allOf", "anyOf", "oneOf", "not", "multipleOf", "const", "enum", "uniqueItems", "pattern", "if", "dependentRequired", "dependentSchemas", "contains", "patternProperties", "propertyNames", "unevaluatedProperties", "unevaluatedItems" -> op;
                    default -> throw malformed(location, "unknown checked opcode");
                };
                boolean normalizedBound = op.equals("bound") && bool(check, "exclusive")
                        && location.equals(child(at, bool(check, "maximum") ? "maximum" : "minimum"));
                if ((!location.equals(keyword.isEmpty() ? at : child(at, keyword)) && !normalizedBound) || !locations.add(location))
                    throw malformed(location, "inconsistent instruction source");
                switch (op) {
                    case "always" -> { bool(check, "value"); if (checks.size() != 1) throw malformed(location, "boolean schema has siblings"); }
                    case "type" -> {
                        Set<String> seen = new HashSet<>();
                        for (JsonValue type : arr(get(check, "types"))) {
                            String name = str(type);
                            if (!Set.of("null", "boolean", "integer", "number", "string", "array", "object").contains(name) || !seen.add(name)) throw malformed(location, "unknown or duplicate type");
                        }
                        if (seen.isEmpty()) throw malformed(location, "empty type list");
                    }
                    case "ref" -> target(check, "target", location);
                    case "dynamicRef" -> resources.checkDynamic(check);
                    case "properties" -> {
                        for (JsonValue item : arr(get(check, "properties"))) {
                            JsonObject property = obj(item); String name = str(property, "name");
                            if (!properties.add(name)) throw malformed(location, "duplicate property");
                            located(target(property, "target", location), child(location, name), location);
                        }
                    }
                    case "additionalProperties", "additionalPropertiesWithPatterns" -> { declared = names(get(check, "declared"), location); declaredNames.put(check, Set.copyOf(declared)); located(target(check, "target", location), location, location); additionalPatterns = op.equals("additionalPropertiesWithPatterns"); additionalAt = location; }
                    case "required" -> names(get(check, "names"), location);
                    case "items" -> { hasItems = true; start = integer(check, "start"); located(target(check, "target", location), location, location); }
                    case "not" -> located(target(check, "target", location), location, location);
                    case "if" -> {
                        located(target(check, "condition", location), location, location);
                        int thenTarget = optionalTarget(check, "thenTarget", location), elseTarget = optionalTarget(check, "elseTarget", location);
                        if (thenTarget >= 0) located(thenTarget, child(at, "then"), location);
                        if (elseTarget >= 0) located(elseTarget, child(at, "else"), location);
                    }
                    case "dependentRequired" -> {
                        var triggers = new HashSet<String>();
                        for (JsonValue entry : arr(get(check, "dependencies"))) {
                            List<JsonValue> pair = arr(entry);
                            if (pair.size() != 2 || !triggers.add(str(pair.get(0)))) throw malformed(location, "invalid dependency trigger");
                            names(pair.get(1), child(location, str(pair.get(0))));
                        }
                    }
                    case "dependentSchemas" -> {
                        var triggers = new HashSet<String>();
                        for (JsonValue entry : arr(get(check, "dependencies"))) {
                            JsonObject dependency = obj(entry); String name = str(dependency, "name");
                            if (!triggers.add(name)) throw malformed(location, "duplicate dependency trigger");
                            located(target(dependency, "target", location), child(location, name), location);
                        }
                    }
                    case "contains" -> {
                        located(target(check, "target", location), location, location);
                        containsBounds.put(check, new ContainsBounds(countOperand(check, "minimum", child(at, "minContains")), countOperand(check, "maximum", child(at, "maxContains"))));
                    }
                    case "patternProperties" -> {
                        var found = new ArrayList<PatternRule>(); var names = new HashSet<String>();
                        for (JsonValue entry : arr(get(check, "patterns"))) {
                            List<JsonValue> triple = arr(entry);
                            if (triple.size() != 3 || !names.add(str(triple.get(0)))) throw malformed(location, "invalid property pattern tuple");
                            String name = str(triple.get(0)); int index = integer(triple.get(2));
                            if (index < 0 || index >= nodes.size()) throw malformed(location, "pattern target outside graph");
                            located(index, child(location, name), location);
                            found.add(new PatternRule(new Pattern(obj(triple.get(1)), child(location, name)), index));
                        }
                        rules = List.copyOf(found);
                    }
                    case "propertyNames", "unevaluatedProperties", "unevaluatedItems" -> located(target(check, "target", location), location, location);
                    case "allOf", "anyOf", "oneOf", "prefixItems" -> {
                        List<JsonValue> targets = arr(get(check, "targets"));
                        if (targets.isEmpty()) throw malformed(location, "empty applicator");
                        if (op.equals("prefixItems")) prefix = targets.size();
                        for (int i = 0; i < targets.size(); i++) {
                            int index = integer(targets.get(i));
                            if (index < 0 || index >= nodes.size()) throw malformed(location, "target outside graph");
                            located(index, child(location, Integer.toString(i)), location);
                        }
                    }
                    case "bound", "multipleOf", "count" -> {
                        String token = str(check, "value");
                        if (token.length() > maxNumberBytes) throw malformed(location, "numeric operand byte ceiling");
                        JsonNumber operand = JsonNumber.parse(token);
                        if (op.equals("multipleOf") && operand.signum() <= 0 || op.equals("count") && (!operand.isInteger() || operand.signum() < 0)) throw malformed(location, "invalid numeric operand");
                        operands.put(check, operand);
                    }
                    case "const" -> get(check, "value");
                    case "enum" -> arr(get(check, "values"));
                    case "pattern" -> patterns.put(check, new Pattern(obj(get(check, "program")), location));
                    case "uniqueItems" -> { }
                    default -> throw malformed(location, "unknown checked opcode");
                }
            }
            if (declared != null && !declared.equals(properties) || hasItems && start != prefix) throw malformed(at, "inconsistent adjacent applicator operands");
            if (declared != null && additionalPatterns != !rules.isEmpty()) throw malformed(additionalAt, "additional-properties pattern exclusions disagree with adjacent patterns");
            patternGroups.put(node, rules);
        }
    }

    private record ContainsBounds(JsonNumber minimum, JsonNumber maximum) {}
    private record PatternRule(Pattern pattern, int target) {}
    private static final Comparator<String> SCALAR_ORDER = (left, right) -> {
        int a = 0, b = 0;
        while (a < left.length() && b < right.length()) {
            int x = left.codePointAt(a), y = right.codePointAt(b);
            if (x != y) return Integer.compare(x, y);
            a += Character.charCount(x); b += Character.charCount(y);
        }
        return Integer.compare(left.length() - a, right.length() - b);
    };
    private static List<String> orderedKeys(JsonObject value) {
        var keys = new ArrayList<>(value.values().keySet()); keys.sort(SCALAR_ORDER); return keys;
    }
    private static final class Scope {
        final Set<String> properties = new TreeSet<>(SCALAR_ORDER);
        final Set<Integer> items = new TreeSet<>();
        void merge(Scope other, Session session, String at, String path) {
            // Charge every candidate, including a duplicate already in the set.
            for (String name : other.properties) { session.spend(1, at, path); properties.add(name); }
            for (int index : other.items) { session.spend(1, at, path); items.add(index); }
        }
    }
    private record Evaluation(boolean valid, Scope scope) {}

    private static CodecException malformed(String source, String message) { return new CodecException("program", source, "", message); }
    private static Set<String> names(JsonValue value, String at) {
        var names = new HashSet<String>();
        for (JsonValue item : arr(value)) if (!names.add(str(item))) throw malformed(at, "duplicate operand name");
        return names;
    }
    private static String checkedSource(JsonObject value) {
        JsonObject location = obj(get(value, "source"));
        String document = str(location, "document"), pointer = str(location, "pointer");
        URI uri = URI.create(document);
        if (!uri.isAbsolute() || uri.getRawFragment() != null || (!pointer.isEmpty() && !pointer.startsWith("/"))) throw malformed("", "invalid source identity");
        for (int i = 0; i < pointer.length(); i++) if (pointer.charAt(i) == '~') {
            if (++i == pointer.length() || pointer.charAt(i) != '0' && pointer.charAt(i) != '1') throw malformed("", "invalid source pointer");
        }
        return document + "#" + pointer;
    }
    static JsonObject obj(JsonValue v) { return (JsonObject) v; }
    static List<JsonValue> arr(JsonValue v) { return ((JsonArray) v).values(); }
    static JsonValue get(JsonObject o, String key) { return Objects.requireNonNull(o.values().get(key)); }
    static String str(JsonValue v) { return ((JsonString) v).value(); }
    static String str(JsonObject o, String key) { return str(get(o, key)); }
    static int integer(JsonValue v) { return ((JsonNumber) v).exactIntegerValue(10).intValueExact(); }
    static int integer(JsonObject o, String key) { return integer(get(o, key)); }
    static boolean bool(JsonObject o, String key) { return ((JsonBoolean) get(o, key)).value(); }
    static String source(JsonObject o) { JsonObject s = obj(get(o, "source")); return str(s, "document") + "#" + str(s, "pointer"); }
    static String child(String path, String key) { return path + "/" + key.replace("~", "~0").replace("/", "~1"); }
    private static String kind(JsonValue v) {
        return switch (v) { case JsonNull n -> "null"; case JsonBoolean b -> "boolean"; case JsonNumber n -> "number"; case JsonString s -> "string"; case JsonArray a -> "array"; case JsonObject o -> "object"; };
    }

    static final class Session {
        private final Program program;
        private long steps, equalities;
        private final LongConsumer bytes;
        private final IdentityHashMap<JsonValue, Set<Integer>> active = new IdentityHashMap<>();
        private record ResourceIdentity(int node, int context) {}
        private final IdentityHashMap<JsonValue, Set<ResourceIdentity>> resourceActive = new IdentityHashMap<>();
        private final ValidationResources.Context resourceContext;
        private CodecException first;
        Session(Program program, long steps, long equalities, LongConsumer bytes) { this.program = program; this.steps = steps; this.equalities = equalities; this.bytes = bytes; resourceContext = program.resources == null ? null : program.resources.context(); }
        void spend(long amount, String source, String path) {
            if (Thread.currentThread().isInterrupted()) throw new CodecException("cancelled", source, path, "interrupted");
            if (amount < 0 || steps < amount) fail(source, path, "evaluation visit ceiling");
            steps -= amount;
        }
        void fail(String source, String path, String message) { throw new CodecException("evaluation_failure", source, path, message); }
        boolean mismatch(String source, String path, String message) { if (first == null) first = new CodecException("invalid", source, path, message); return false; }
        void check(int root, JsonValue value, String path) {
            first = null;
            if (!program.rootIndices.contains(root)) fail("", path, "root was not selected");
            if (!eval(root, Objects.requireNonNull(value), path, 0)) throw first == null ? new CodecException("invalid", program.sourceAt(root), path, "source schema rejected value") : first;
        }
        boolean matches(int root, JsonValue value, String path) {
            if (!program.rootIndices.contains(root)) fail("", path, "branch root was not selected");
            CodecException saved = first;
            try { return eval(root, value, path, 0); } finally { first = saved; }
        }
        JsonNumber number(JsonValue value, String at, String path) {
            JsonNumber number = (JsonNumber) value;
            if (number.token().length() > program.maxNumberBytes) fail(at, path, "numeric operand byte ceiling");
            bytes.accept(number.token().length());
            return number;
        }
        int compare(JsonNumber left, JsonNumber right) { bytes.accept((long) left.token().length() + right.token().length()); return left.compareTo(right); }
        boolean eval(int index, JsonValue value, String path, int depth) {
            if (program.scoped) return evalScoped(index, value, path, depth).valid();
            JsonObject node = obj(program.nodes.get(index)); String at = source(node);
            spend(1, at, path);
            if (depth >= program.maxDepth) fail(at, path, "validation depth ceiling");
            Set<Integer> indices = active.computeIfAbsent(value, ignored -> new HashSet<>());
            if (!indices.add(index)) fail(at, path, "nonproductive recursive evaluation");
            try {
                boolean valid = true;
                for (JsonValue raw : arr(get(node, "checks"))) {
                    JsonObject check = obj(raw); at = source(check); spend(1, at, path);
                    String op = str(check, "op"); boolean result = true;
                    switch (op) {
                        case "ref" -> result = eval(integer(check, "target"), value, path, depth + 1);
                        case "allOf", "anyOf", "oneOf" -> {
                            int count = 0; List<JsonValue> targets = arr(get(check, "targets"));
                            for (JsonValue target : targets) {
                                spend(1, at, path); CodecException saved = first;
                                boolean matched = eval(integer(target), value, path, depth + 1);
                                if (!op.equals("allOf")) first = saved;
                                if (matched) count++;
                            }
                            result = op.equals("allOf") ? count == targets.size() : op.equals("anyOf") ? count > 0 : count == 1;
                        }
                        case "not" -> { CodecException saved = first; result = !eval(integer(check, "target"), value, path, depth + 1); first = saved; }
                        case "properties" -> {
                            if (value instanceof JsonObject object) for (JsonValue entry : arr(get(check, "properties"))) {
                                spend(1, at, path); JsonObject p = obj(entry); String name = str(p, "name"); bytes.accept(name.length());
                                if (object.values().containsKey(name)) result = eval(integer(p, "target"), object.values().get(name), child(path, name), depth + 1) & result;
                            }
                        }
                        case "additionalProperties" -> {
                            if (value instanceof JsonObject object) {
                                Set<String> declared = program.declaredNames.get(check);
                                for (var entry : object.values().entrySet()) {
                                    spend(1, at, path); bytes.accept(entry.getKey().length());
                                    if (!declared.contains(entry.getKey())) result = eval(integer(check, "target"), entry.getValue(), child(path, entry.getKey()), depth + 1) & result;
                                }
                            }
                        }
                        case "items", "prefixItems" -> {
                            if (value instanceof JsonArray array) {
                                List<JsonValue> targets = op.equals("prefixItems") ? arr(get(check, "targets")) : List.of();
                                int start = op.equals("items") ? integer(check, "start") : 0;
                                int end = op.equals("items") ? array.values().size() : Math.min(targets.size(), array.values().size());
                                for (int i = start; i < end; i++) {
                                    spend(1, at, path);
                                    result = eval(op.equals("items") ? integer(check, "target") : integer(targets.get(i)), array.values().get(i), child(path, Integer.toString(i)), depth + 1) & result;
                                }
                            }
                        }
                        default -> result = scalar(check, value, path);
                    }
                    if (!result) mismatch(at, path, op + " assertion rejected value");
                    valid = result & valid;
                }
                return valid;
            } finally { indices.remove(index); if (indices.isEmpty()) active.remove(value); }
        }
        private Evaluation evalScoped(int index, JsonValue value, String path, int depth) {
            JsonObject node = obj(program.nodes.get(index)); String at = source(node);
            spend(1, at, path);
            if (depth >= program.maxDepth) fail(at, path, "validation depth ceiling");
            int entered = resourceContext == null ? -1 : resourceContext.enter(index, this, at, path);
            Set<Integer> indices = null; Set<ResourceIdentity> identities = null; ResourceIdentity identity = null; boolean added = false;
            try {
                if (resourceContext == null) { indices = active.computeIfAbsent(value, ignored -> new HashSet<>()); added = indices.add(index); }
                else { identities = resourceActive.computeIfAbsent(value, ignored -> new HashSet<>()); identity = new ResourceIdentity(index, resourceContext.identity()); added = identities.add(identity); }
                if (!added) fail(at, path, "nonproductive recursive evaluation");
                Scope local = new Scope(); boolean valid = true;
                for (JsonValue raw : arr(get(node, "checks"))) {
                    JsonObject check = obj(raw); at = source(check); spend(1, at, path);
                    Evaluation result = applyScoped(node, check, value, path, depth, local);
                    if (result.valid()) local.merge(result.scope(), this, at, path);
                    else mismatch(at, path, str(check, "op") + " assertion rejected value");
                    valid &= result.valid();
                }
                return new Evaluation(valid, valid ? local : new Scope());
            } finally {
                if (added) {
                    if (resourceContext == null) { indices.remove(index); if (indices.isEmpty()) active.remove(value); }
                    else { identities.remove(identity); if (identities.isEmpty()) resourceActive.remove(value); }
                }
                if (resourceContext != null) resourceContext.leave(entered);
            }
        }
        private Evaluation trialScoped(int index, JsonValue value, String path, int depth) {
            CodecException saved = first;
            try { return evalScoped(index, value, path, depth); }
            finally { first = saved; }
        }
        private Evaluation composeScoped(JsonObject check, JsonValue value, String path, int depth) {
            String at = source(check), op = str(check, "op");
            List<JsonValue> targets = arr(get(check, "targets")); var passing = new ArrayList<Scope>();
            boolean all = true;
            for (JsonValue target : targets) {
                spend(1, at, path);
                Evaluation result = op.equals("allOf") ? evalScoped(integer(target), value, path, depth + 1) : trialScoped(integer(target), value, path, depth + 1);
                all &= result.valid(); if (result.valid()) passing.add(result.scope());
            }
            boolean valid = op.equals("allOf") ? all : op.equals("anyOf") ? !passing.isEmpty() : passing.size() == 1;
            Scope produced = new Scope();
            if (valid) for (Scope branch : passing) produced.merge(branch, this, at, path);
            return new Evaluation(valid, produced);
        }
        private Evaluation applyScoped(JsonObject node, JsonObject check, JsonValue value, String path, int depth, Scope local) {
            String op = str(check, "op"), at = source(check); Scope produced = new Scope(); boolean valid = true;
            switch (op) {
                case "ref" -> { return evalScoped(integer(check, "target"), value, path, depth + 1); }
                case "dynamicRef" -> { return evalScoped(resourceContext.resolve(check, this, at, path), value, path, depth + 1); }
                case "allOf", "anyOf", "oneOf" -> { return composeScoped(check, value, path, depth); }
                case "not" -> valid = !trialScoped(integer(check, "target"), value, path, depth + 1).valid();
                case "if" -> {
                    Evaluation condition = trialScoped(integer(check, "condition"), value, path, depth + 1);
                    if (condition.valid()) local.merge(condition.scope(), this, at, path);
                    JsonValue selected = get(check, condition.valid() ? "thenTarget" : "elseTarget");
                    // The branch receives no condition/caller/sibling annotations.
                    if (selected != JsonNull.INSTANCE) return evalScoped(integer(selected), value, path, depth + 1);
                }
                case "properties" -> {
                    if (value instanceof JsonObject object) for (JsonValue raw : arr(get(check, "properties"))) {
                        spend(1, at, path); JsonObject property = obj(raw); String name = str(property, "name");
                        if (object.values().containsKey(name)) {
                            valid &= evalScoped(integer(property, "target"), object.values().get(name), child(path, name), depth + 1).valid();
                            produced.properties.add(name);
                        }
                    }
                }
                case "additionalProperties", "additionalPropertiesWithPatterns" -> {
                    if (value instanceof JsonObject object) {
                        Set<String> declared = program.declaredNames.get(check);
                        for (String name : orderedKeys(object)) {
                            spend(1, at, path); if (declared.contains(name)) continue;
                            boolean excluded = false;
                            if (op.equals("additionalPropertiesWithPatterns")) {
                                for (PatternRule rule : program.patternGroups.get(node)) {
                                    spend(1, at, path);
                                    if (rule.pattern().matches(name, this, at, path)) { excluded = true; break; }
                                }
                            }
                            if (!excluded) {
                                valid &= evalScoped(integer(check, "target"), object.values().get(name), child(path, name), depth + 1).valid();
                                produced.properties.add(name);
                            }
                        }
                    }
                }
                case "items", "prefixItems" -> {
                    if (value instanceof JsonArray array) {
                        List<JsonValue> targets = op.equals("prefixItems") ? arr(get(check, "targets")) : List.of();
                        int start = op.equals("items") ? integer(check, "start") : 0;
                        int end = op.equals("items") ? array.values().size() : Math.min(targets.size(), array.values().size());
                        for (int index = start; index < end; index++) {
                            spend(1, at, path);
                            valid &= evalScoped(op.equals("items") ? integer(check, "target") : integer(targets.get(index)), array.values().get(index), child(path, Integer.toString(index)), depth + 1).valid();
                            produced.items.add(index);
                        }
                    }
                }
                case "dependentRequired" -> {
                    if (value instanceof JsonObject object) for (JsonValue entry : arr(get(check, "dependencies"))) {
                        spend(1, at, path); List<JsonValue> dependency = arr(entry); String trigger = str(dependency.get(0));
                        if (object.values().containsKey(trigger)) for (JsonValue name : arr(dependency.get(1))) {
                            spend(1, at, path);
                            if (!object.values().containsKey(str(name))) { mismatch(child(at, trigger), path, "dependent required property is absent"); valid = false; }
                        }
                    }
                }
                case "dependentSchemas" -> {
                    var passing = new ArrayList<Scope>();
                    if (value instanceof JsonObject object) for (JsonValue entry : arr(get(check, "dependencies"))) {
                        spend(1, at, path); JsonObject dependency = obj(entry); String trigger = str(dependency, "name");
                        if (object.values().containsKey(trigger)) {
                            Evaluation result = evalScoped(integer(dependency, "target"), value, path, depth + 1);
                            valid &= result.valid(); if (result.valid()) passing.add(result.scope());
                        }
                    }
                    if (valid) for (Scope branch : passing) produced.merge(branch, this, at, path);
                }
                case "contains" -> {
                    if (value instanceof JsonArray array) {
                        int matches = 0;
                        for (int index = 0; index < array.values().size(); index++) {
                            spend(1, at, path);
                            if (trialScoped(integer(check, "target"), array.values().get(index), child(path, Integer.toString(index)), depth + 1).valid()) {
                                matches++; produced.items.add(index);
                            }
                        }
                        ContainsBounds bounds = program.containsBounds.get(check);
                        if (matches != 0 || bounds.minimum() != null && bounds.minimum().signum() == 0) local.merge(produced, this, at, path);
                        produced = new Scope();
                        // The implicit minimum 1 is semantic, never a fabricated
                        // numeric operand that could violate maxNumberBytes=0.
                        boolean lower = bounds.minimum() == null ? matches >= 1 : compare(JsonNumber.of(matches), bounds.minimum()) >= 0;
                        boolean upper = bounds.maximum() == null || compare(JsonNumber.of(matches), bounds.maximum()) <= 0;
                        if (!lower) mismatch(bounds.minimum() == null ? at : child(source(node), "minContains"), path, "fewer contains matches than required");
                        if (!upper) mismatch(child(source(node), "maxContains"), path, "more contains matches than allowed");
                        valid = lower && upper;
                    }
                }
                case "patternProperties" -> {
                    if (value instanceof JsonObject object) for (String name : orderedKeys(object)) {
                        spend(1, at, path);
                        for (PatternRule rule : program.patternGroups.get(node)) {
                            spend(1, at, path);
                            if (rule.pattern().matches(name, this, at, path)) {
                                valid &= evalScoped(rule.target(), object.values().get(name), child(path, name), depth + 1).valid();
                                produced.properties.add(name);
                            }
                        }
                    }
                }
                case "propertyNames" -> {
                    if (value instanceof JsonObject object) for (String name : orderedKeys(object)) {
                        spend(1, at, path);
                        JsonString key = new JsonString(name);
                        valid &= evalScoped(integer(check, "target"), key, child(path, name), depth + 1).valid();
                    }
                }
                case "unevaluatedProperties" -> {
                    if (value instanceof JsonObject object) for (String name : orderedKeys(object)) {
                        spend(1, at, path);
                        if (!local.properties.contains(name)) {
                            valid &= evalScoped(integer(check, "target"), object.values().get(name), child(path, name), depth + 1).valid();
                            produced.properties.add(name);
                        }
                    }
                }
                case "unevaluatedItems" -> {
                    if (value instanceof JsonArray array) for (int index = 0; index < array.values().size(); index++) {
                        spend(1, at, path);
                        if (!local.items.contains(index)) {
                            valid &= evalScoped(integer(check, "target"), array.values().get(index), child(path, Integer.toString(index)), depth + 1).valid();
                            produced.items.add(index);
                        }
                    }
                }
                default -> valid = scalar(check, value, path);
            }
            return new Evaluation(valid, produced);
        }
        boolean equal(JsonValue left, JsonValue right, String at, String path, int depth) {
            if (equalities == 0 || depth >= program.maxDepth) fail(at, path, "equality ceiling");
            equalities--;
            if (!kind(left).equals(kind(right))) return false;
            if (left instanceof JsonNumber) return compare(number(left, at, path), number(right, at, path)) == 0;
            if (left instanceof JsonArray a && right instanceof JsonArray b) {
                if (a.values().size() != b.values().size()) return false;
                for (int i = 0; i < a.values().size(); i++) if (!equal(a.values().get(i), b.values().get(i), at, path, depth + 1)) return false;
                return true;
            }
            if (left instanceof JsonObject a && right instanceof JsonObject b) {
                if (a.values().size() != b.values().size()) return false;
                for (var entry : a.values().entrySet()) {
                    bytes.accept(entry.getKey().length());
                    JsonValue other = b.values().get(entry.getKey());
                    if (other == null || !equal(entry.getValue(), other, at, path, depth + 1)) return false;
                }
                return true;
            }
            if (left instanceof JsonString a && right instanceof JsonString b) bytes.accept((long) a.value().length() + b.value().length());
            return left.equals(right);
        }
        boolean scalar(JsonObject check, JsonValue value, String path) {
            String op = str(check, "op"), at = source(check), kind = kind(value);
            switch (op) {
                case "always": return bool(check, "value");
                case "type": {
                    for (JsonValue type : arr(get(check, "types"))) if (str(type).equals(kind)
                            || str(type).equals("integer") && value instanceof JsonNumber && number(value, at, path).isInteger()) return true;
                    return false;
                }
                case "required": {
                    boolean result = true;
                    if (value instanceof JsonObject object) for (JsonValue name : arr(get(check, "names"))) {
                        spend(1, at, path); bytes.accept(str(name).length());
                        if (!object.values().containsKey(str(name))) result = false;
                    }
                    return result;
                }
                case "bound": {
                    if (!(value instanceof JsonNumber)) return true;
                    int order = compare(number(value, at, path), program.operands.get(check));
                    return bool(check, "maximum") ? (bool(check, "exclusive") ? order < 0 : order <= 0) : (bool(check, "exclusive") ? order > 0 : order >= 0);
                }
                case "multipleOf": return !(value instanceof JsonNumber) || number(value, at, path).multipleOf(program.operands.get(check), bytes);
                case "count": {
                    if (!str(check, "target").equals(kind)) return true;
                    int count = switch (value) {
                        case JsonString s -> { bytes.accept(s.value().length()); yield s.value().codePointCount(0, s.value().length()); }
                        case JsonArray a -> a.values().size(); case JsonObject o -> o.values().size();
                        default -> throw malformed(at, "invalid count target");
                    };
                    int order = compare(JsonNumber.of(count), program.operands.get(check));
                    return bool(check, "maximum") ? order <= 0 : order >= 0;
                }
                case "const": return equal(value, get(check, "value"), at, path, 0);
                case "enum": {
                    for (JsonValue candidate : arr(get(check, "values"))) { spend(1, at, path); if (equal(value, candidate, at, path, 0)) return true; }
                    return false;
                }
                case "uniqueItems": {
                    if (value instanceof JsonArray array) for (int i = 0; i < array.values().size(); i++) {
                        spend(1, at, path);
                        for (int j = 0; j < i; j++) { spend(1, at, path); if (equal(array.values().get(i), array.values().get(j), at, path, 0)) return false; }
                    }
                    return true;
                }
                case "pattern": return !(value instanceof JsonString s) || program.patterns.get(check).matches(s.value(), this, at, path);
                default: throw malformed(at, "unknown checked instruction");
            }
        }
    }

    private record State(String op, int target, int second, int[][] ranges) {}
    private static final class Pattern {
        final State[] states;
        final int start;
        Pattern(JsonObject raw, String at) {
            if (!str(raw, "version").equals("suspect.pattern.experimental.v1")) throw malformed(at, "unsupported pattern version");
            List<JsonValue> values = arr(get(raw, "states"));
            if (values.isEmpty() || values.size() > 8192) throw malformed(at, "pattern state ceiling");
            states = new State[values.size()]; start = edge(integer(raw, "start"), at);
            int totalRanges = 0;
            for (int i = 0; i < states.length; i++) {
                JsonObject state = obj(values.get(i)); String op = str(state, "op"); int target = 0, second = 0; int[][] ranges = new int[0][];
                switch (op) {
                    case "match" -> { }
                    case "split" -> { target = edge(integer(state, "first"), at); second = edge(integer(state, "second"), at); }
                    case "jump", "start", "end" -> target = edge(integer(state, "target"), at);
                    case "char" -> {
                        target = edge(integer(state, "target"), at); List<JsonValue> entries = arr(get(state, "ranges"));
                        totalRanges += entries.size(); if (totalRanges > 65536) throw malformed(at, "pattern range ceiling");
                        ranges = new int[entries.size()][]; int previous = -1;
                        for (int n = 0; n < entries.size(); n++) {
                            List<JsonValue> range = arr(entries.get(n)); if (range.size() != 2) throw malformed(at, "invalid pattern range");
                            int low = integer(range.get(0)), high = integer(range.get(1));
                            if (low <= previous || high < low || high > 0x10ffff || low <= 0xdfff && high >= 0xd800) throw malformed(at, "invalid Unicode scalar range");
                            ranges[n] = new int[]{low, high}; previous = high;
                        }
                    }
                    default -> throw malformed(at, "unknown pattern instruction");
                }
                states[i] = new State(op, target, second, ranges);
            }
        }
        int edge(int index, String at) { if (index < 0 || index >= states.length) throw malformed(at, "pattern target outside graph"); return index; }
        boolean matches(String value, Session session, String at, String path) {
            // Thompson NFA: add the unanchored start at each position and merge
            // active states. No suffix rescanning or backtracking Java regex.
            int[] seen = new int[states.length]; int epoch = 0;
            var seeds = new ArrayList<Integer>(); var next = new ArrayList<Integer>();
            var pending = new ArrayDeque<Integer>(); var consuming = new ArrayList<Integer>();
            for (int offset = 0; ; ) {
                session.spend(1, at, path); epoch++;
                pending.clear(); consuming.clear(); pending.add(start); pending.addAll(seeds);
                while (!pending.isEmpty()) {
                    session.spend(1, at, path); int index = pending.removeLast();
                    if (seen[index] == epoch) continue; seen[index] = epoch;
                    State state = states[index];
                    switch (state.op()) {
                        case "match": return true;
                        case "split": pending.add(state.target()); pending.add(state.second()); break;
                        case "jump": pending.add(state.target()); break;
                        case "start": if (offset == 0) pending.add(state.target()); break;
                        case "end": if (offset == value.length()) pending.add(state.target()); break;
                        case "char": consuming.add(index); break;
                        default: throw malformed(at, "unknown pattern instruction");
                    }
                }
                if (offset == value.length()) return false;
                int scalar = value.codePointAt(offset); offset += Character.charCount(scalar); next.clear();
                for (int index : consuming) {
                    State state = states[index];
                    for (int[] range : state.ranges()) {
                        session.spend(1, at, path);
                        if (scalar < range[0]) break;
                        if (scalar <= range[1]) { next.add(state.target()); break; }
                    }
                }
                var swap = seeds; seeds = next; next = swap;
            }
        }
    }
}
