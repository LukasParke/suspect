package {package};

import java.io.ByteArrayOutputStream;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.charset.*;
import java.util.*;
import static {package}.JsonRuntime.*;
import static {package}.Validation.*;

/** Checked V3 resource metadata and exact entered-context identity. No acquisition. */
final class ValidationResources {
    private record Source(String document, String pointer) {
        String identity() { return document + "#" + pointer; }
        Source child(String token) { return new Source(document, Validation.child(pointer, token)); }
        boolean contains(Source other) { return document.equals(other.document) && (pointer.equals(other.pointer) || other.pointer.startsWith(pointer + "/")); }
    }
    private record Binding(String name, Source source, int target) {}
    private record Resource(Source source, String kind, String base, List<Binding> bindings) {}
    final int[] nodeResources;
    private final List<Resource> resources;

    ValidationResources(JsonObject raw, List<JsonValue> nodes) {
        List<JsonValue> entries = arr(get(raw, "resources")), scopes = arr(get(raw, "nodeScopes"));
        if (entries.size() > 100_000 || scopes.size() != nodes.size()) throw bad("", "resource/node alignment ceiling");
        List<Resource> resources = new ArrayList<>(); Set<Source> sources = new HashSet<>(); Map<String, Integer> aliases = new HashMap<>();
        for (int index = 0; index < entries.size(); index++) {
            JsonObject entry = obj(entries.get(index)); Source at = sourceValue(get(entry, "source")); String kind = str(entry, "kind");
            if (!sources.add(at)) throw bad(at.identity(), "duplicate physical resource");
            if (!Set.of("schema", "document", "openApiDocument").contains(kind)) throw bad(at.identity(), "unknown resource kind");
            String canonical = uriKey(str(entry, "canonicalUri"), at.identity()); String base = str(entry, "baseUri");
            if (base.indexOf('#') >= 0 || !uriKey(base, at.identity()).equals(base) || !document(str(entry, "canonicalUri"), at.identity()).equals(base)
                    || !kind.equals("openApiDocument") && !canonical.equals(base)) throw bad(at.identity(), "inconsistent canonical resource/base URI");
            JsonValue declaration = get(entry, "declarationSource");
            if (declaration != JsonNull.INSTANCE) {
                Source source = sourceValue(declaration); String keyword = kind.equals("schema") ? "$id" : kind.equals("openApiDocument") ? "$self" : null;
                if (keyword == null || !source.equals(at.child(keyword))) throw bad(source.identity(), "identifier declaration is not at resource boundary");
            } else if (!at.pointer().isEmpty() || !canonical.equals(uriKey(at.document(), at.identity()))) throw bad(at.identity(), "undeclared resource changed retrieval identity");
            Set<String> spellings = new HashSet<>(), names = new HashSet<>();
            for (JsonValue value : arr(get(entry, "aliases"))) {
                String alias = str(value); if (!spellings.add(alias)) throw bad(at.identity(), "duplicate resource alias");
                String key = uriKey(alias, at.identity()); Integer previous = aliases.putIfAbsent(key, index);
                if (previous != null && previous != index) throw bad(at.identity(), "URI alias identifies multiple physical resources");
                names.add(key);
            }
            if (!names.contains(canonical) || !names.contains(base)) throw bad(at.identity(), "aliases omit canonical identifier/base");
            List<Binding> bindings = new ArrayList<>(); Set<String> anchors = new HashSet<>();
            for (JsonValue value : arr(get(entry, "dynamicAnchors"))) {
                List<JsonValue> triple = arr(value); if (triple.size() != 3) throw bad(at.identity(), "invalid dynamic anchor tuple");
                String name = str(triple.get(0)); Source source = sourceValue(triple.get(1)); int target = index(triple.get(2), nodes.size(), source.identity());
                if (!anchor(name) || !anchors.add(name)) throw bad(source.identity(), "invalid or duplicate dynamic anchor name");
                bindings.add(new Binding(name, source, target));
            }
            resources.add(new Resource(at, kind, base, List.copyOf(bindings)));
        }
        nodeResources = new int[nodes.size()]; Set<Integer> used = new HashSet<>();
        for (int index = 0; index < nodes.size(); index++) {
            Source node = sourceValue(get(obj(nodes.get(index)), "source")); List<JsonValue> triple = arr(scopes.get(index));
            if (triple.size() != 3) throw bad(node.identity(), "invalid node resource scope tuple");
            int resourceIndex = index(triple.get(0), resources.size(), node.identity()); Resource resource = resources.get(resourceIndex); Source schemaRoot = sourceValue(triple.get(1));
            if (!resource.source().contains(schemaRoot) || !schemaRoot.contains(node) || resource.kind().equals("schema") && !schemaRoot.equals(resource.source())) throw bad(node.identity(), "node/resource/schema roots do not contain physical source");
            String suffix = node.pointer().substring(resource.source().pointer().length()); String address = resource.base() + (suffix.isEmpty() ? "" : "#" + encodeFragment(suffix));
            if (!str(triple.get(2)).equals(address)) throw bad(node.identity(), "canonical node address differs from indexed source pointer");
            nodeResources[index] = resourceIndex; used.add(resourceIndex);
        }
        if (used.size() != resources.size()) throw bad("", "unreferenced compiled resource");
        for (int index = 0; index < resources.size(); index++) for (Binding binding : resources.get(index).bindings()) {
            Source node = sourceValue(get(obj(nodes.get(binding.target())), "source"));
            if (!binding.source().equals(node.child("$dynamicAnchor")) || nodeResources[binding.target()] != index) throw bad(binding.source().identity(), "dynamic binding target/source/resource mismatch");
        }
        this.resources = List.copyOf(resources);
    }
    void checkDynamic(JsonObject check) {
        String at = Validation.source(check); int target = index(get(check, "target"), nodeResources.length, at), initial = index(get(check, "initialResource"), resources.size(), at);
        if (nodeResources[target] != initial) throw bad(at, "dynamic fallback resource disagrees with target scope");
        JsonValue value = get(check, "anchor");
        if (value != JsonNull.INSTANCE) {
            String name = str(value);
            if (!anchor(name) || resources.get(initial).bindings().stream().noneMatch(binding -> binding.name().equals(name) && binding.target() == target)) throw bad(at, "initial dynamic name does not bind the fallback target");
        }
    }
    Context context() { return new Context(); }
    private record ContextEdge(int previous, int resource) {}
    final class Context {
        private final List<Integer> order = new ArrayList<>();
        private final boolean[] entered = new boolean[resources.size()];
        private final Map<ContextEdge, Integer> interned = new HashMap<>();
        private int identity;
        int identity() { return identity; }
        int enter(int node, Validation.Session session, String at, String path) {
            int resource = nodeResources[node]; if (entered[resource]) return -1;
            session.spend(1, at, path); int previous = identity;
            // Exact edge equality, not a hash-only approximation of a context.
            identity = interned.computeIfAbsent(new ContextEdge(previous, resource), ignored -> interned.size() + 1);
            entered[resource] = true; order.add(resource); return previous;
        }
        void leave(int previous) {
            if (previous < 0) return;
            int resource = order.removeLast(); entered[resource] = false; identity = previous;
        }
        int resolve(JsonObject check, Validation.Session session, String at, String path) {
            int fallback = integer(check, "target"); JsonValue anchor = get(check, "anchor");
            if (anchor == JsonNull.INSTANCE) return fallback;
            String name = str(anchor);
            for (int resource : order) {
                session.spend(1, at, path);
                for (Binding binding : resources.get(resource).bindings()) {
                    session.spend(1, at, path); if (binding.name().equals(name)) return binding.target();
                }
            }
            return fallback;
        }
    }
    private static int index(JsonValue value, int size, String at) {
        int result; try { result = integer(value); } catch (ArithmeticException | ClassCastException error) { throw bad(at, "resource target must be an integer"); }
        if (result < 0 || result >= size) throw bad(at, "resource target outside registry"); return result;
    }
    private static Source sourceValue(JsonValue value) {
        JsonObject object = obj(value); String document = str(object, "document"), pointer = str(object, "pointer");
        if (document.indexOf('#') >= 0) throw bad("", "physical source document contains a fragment"); document(document, "");
        if (!pointer.isEmpty() && !pointer.startsWith("/")) throw bad("", "source pointer must be empty or absolute");
        for (int i = 0; i < pointer.length(); i++) if (pointer.charAt(i) == '~' && (++i >= pointer.length() || pointer.charAt(i) != '0' && pointer.charAt(i) != '1')) throw bad("", "invalid physical source pointer escape");
        return new Source(document, pointer);
    }
    private static boolean anchor(String value) { return value.matches("[A-Za-z_][A-Za-z0-9_.-]*"); }
    private static CodecException bad(String source, String message) { return new CodecException("program", source, "", message); }

    // RFC 3986 metadata normalization only. Evaluation never resolves a URI or
    // loads a resource. Encoded path octets are retained; fragment text is decoded once.
    private static String uriKey(String value, String at) {
        String document = document(value, at); int hash = value.indexOf('#');
        String fragment = hash < 0 ? "" : decodeFragment(value.substring(hash + 1), at);
        return document + (fragment.isEmpty() ? "" : "#" + encodeFragment(fragment));
    }
    private static String document(String value, String at) {
        if (value.isEmpty() || value.chars().anyMatch(c -> c < 33 || c > 126)) throw bad(at, "resource URI must use absolute RFC 3986 syntax");
        int colon = value.indexOf(':'); if (colon <= 0 || !value.substring(0, colon).matches("[A-Za-z][A-Za-z0-9+.-]*")) throw bad(at, "invalid resource URI scheme");
        int hash = value.indexOf('#'); if (hash >= 0) component(value.substring(hash + 1), "/?", at);
        String rest = value.substring(colon + 1, hash < 0 ? value.length() : hash), authority = null, query = null;
        int question = rest.indexOf('?'); if (question >= 0) { query = rest.substring(question + 1); component(query, "/?", at); rest = rest.substring(0, question); }
        if (rest.startsWith("//")) { int end = rest.indexOf('/', 2); if (end < 0) end = rest.length(); authority = authority(rest.substring(2, end), at); rest = rest.substring(end); }
        component(rest, "/", at); String path = removeDots(rest);
        if (authority == null && path.startsWith("//")) throw bad(at, "unrepresentable authorityless resource path");
        return value.substring(0, colon).toLowerCase(Locale.ROOT) + ":" + (authority == null ? "" : "//" + authority) + path + (query == null ? "" : "?" + query);
    }
    private static String authority(String value, String at) {
        String user = ""; int marker = value.lastIndexOf('@');
        if (marker >= 0) { user = value.substring(0, marker); component(user, "", at); if (user.indexOf('@') >= 0) throw bad(at, "invalid resource userinfo"); user += "@"; value = value.substring(marker + 1); }
        String host, port = "";
        if (value.startsWith("[")) {
            int end = value.indexOf(']'); if (end < 0) throw bad(at, "invalid IP literal"); host = value.substring(0, end + 1); port = value.substring(end + 1);
            String literal = host.substring(1, host.length() - 1);
            if (literal.matches("[vV][0-9A-Fa-f]+\\.[A-Za-z0-9._~!$&'()*+,;=:-]+")) { }
            else try { if (!literal.contains(":") || literal.contains("%")) throw new IllegalArgumentException(); URI.create("x://" + host + "/"); } catch (IllegalArgumentException error) { throw bad(at, "invalid IP literal"); }
        } else {
            int colon = value.lastIndexOf(':'); host = colon < 0 ? value : value.substring(0, colon); port = colon < 0 ? "" : value.substring(colon);
            component(host, "", at); if (host.indexOf(':') >= 0 || host.indexOf('@') >= 0) throw bad(at, "invalid resource host");
        }
        if (!port.isEmpty() && !port.matches(":[0-9]*")) throw bad(at, "invalid resource URI port");
        return user + host.toLowerCase(Locale.ROOT) + port;
    }
    private static void component(String value, String extra, String at) {
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            if (c == '%') { if (i + 2 >= value.length() || hex(value.charAt(i + 1)) < 0 || hex(value.charAt(i + 2)) < 0) throw bad(at, "invalid resource URI percent escape"); i += 2; }
            else if (!(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || "-._~!$&'()*+,;=:@".indexOf(c) >= 0 || extra.indexOf(c) >= 0)) throw bad(at, "invalid resource URI component");
        }
    }
    private static int hex(int value) { return value < 128 ? Character.digit((char)value, 16) : -1; }
    private static String decodeFragment(String value, String at) {
        var bytes = new ByteArrayOutputStream();
        for (int i = 0; i < value.length(); i++) { char c = value.charAt(i); if (c == '%') { if (i + 2 >= value.length() || hex(value.charAt(i + 1)) < 0 || hex(value.charAt(i + 2)) < 0) throw bad(at, "invalid resource fragment escape"); bytes.write(hex(value.charAt(++i)) * 16 + hex(value.charAt(++i))); } else bytes.write(c); }
        try { return StandardCharsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes.toByteArray())).toString(); }
        catch (CharacterCodingException error) { throw bad(at, "resource fragment is not UTF-8"); }
    }
    private static String encodeFragment(String value) {
        StringBuilder out = new StringBuilder();
        for (byte next : value.getBytes(StandardCharsets.UTF_8)) { int b = next & 255; if (b >= 'a' && b <= 'z' || b >= 'A' && b <= 'Z' || b >= '0' && b <= '9' || "-._~!$&'()*+,;=:@/?".indexOf(b) >= 0) out.append((char)b); else out.append('%').append("0123456789ABCDEF".charAt(b >> 4)).append("0123456789ABCDEF".charAt(b & 15)); }
        return out.toString();
    }
    private static String removeDots(String input) {
        StringBuilder output = new StringBuilder(); int i = 0;
        while (i < input.length()) {
            int remaining = input.length() - i;
            if (input.startsWith("../", i)) i += 3;
            else if (input.startsWith("./", i)) i += 2;
            else if (input.startsWith("/./", i)) i += 2;
            else if (remaining == 2 && input.startsWith("/.", i)) { output.append('/'); break; }
            else if (input.startsWith("/../", i) || remaining == 3 && input.startsWith("/..", i)) {
                int end = output.lastIndexOf("/"); output.setLength(Math.max(0, end));
                if (remaining == 3) { output.append('/'); break; } i += 3;
            } else if (remaining == 1 && input.charAt(i) == '.' || remaining == 2 && input.startsWith("..", i)) break;
            else { int end = input.indexOf('/', i + (input.charAt(i) == '/' ? 1 : 0)); if (end < 0) end = input.length(); output.append(input, i, end); i = end; }
        }
        return output.toString();
    }
}
