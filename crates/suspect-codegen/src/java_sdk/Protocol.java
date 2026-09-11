package {package};

import java.io.IOException;
import java.util.*;
import static {package}.JsonRuntime.*;

/** Immutable compiler-produced HTTP instructions; never an OpenAPI interpreter. */
final class Protocol {
    private Protocol() {}
    private static final JsonObject DATA = load();
    private static JsonObject load() {
        try (var stream = Protocol.class.getResourceAsStream("protocol-program.json")) {
            if (stream == null) throw new IllegalStateException("missing HTTP protocol program");
            JsonObject value = object(JsonRuntime.parse(stream.readNBytes(MAX_BYTES + 1)));
            if (number(get(value,"version")) != 1) throw new IllegalStateException("unsupported HTTP protocol program");
            return value;
        } catch (IOException error) { throw new IllegalStateException("cannot load HTTP protocol program"); }
    }
    static JsonValue at(String pointer) {
        JsonValue value = DATA;
        if (pointer.isEmpty()) return value;
        for (String token : pointer.substring(1).split("/",-1)) {
            token = token.replace("~1","/").replace("~0","~");
            value = value instanceof JsonArray array ? array.values().get(Integer.parseInt(token)) : get(object(value),token);
        }
        return value;
    }
    static JsonObject object(String pointer) { return object(at(pointer)); }
    static JsonObject object(JsonValue value) { return (JsonObject)value; }
    static JsonValue get(JsonValue value,String key) { return value instanceof JsonObject o ? o.values().getOrDefault(key,JsonNull.INSTANCE) : JsonNull.INSTANCE; }
    static List<JsonValue> array(JsonValue value) { return value instanceof JsonArray array ? array.values() : List.of(); }
    static String text(JsonValue value) { return ((JsonString)value).value(); }
    static String text(JsonValue value,String key) { return text(get(value,key)); }
    static String optionalText(JsonValue value,String key) { JsonValue v=get(value,key); return v==JsonNull.INSTANCE ? null : text(v); }
    static boolean flag(JsonValue value,String key) { return get(value,key) instanceof JsonBoolean b && b.value(); }
    static long number(JsonValue value) { return ((JsonNumber)value).exactIntegerValue(19).longValueExact(); }
    static long located(JsonValue value,String key,long fallback) { JsonValue at=get(value,key); return at==JsonNull.INSTANCE?fallback:number(get(at,"value")); }
    static String source(JsonValue value) {
        JsonValue at=get(value,"source");
        if (get(at,"use_site")!=JsonNull.INSTANCE) at=get(get(at,"use_site"),"source");
        else if (get(at,"source")!=JsonNull.INSTANCE) at=get(at,"source");
        return get(at,"document") instanceof JsonString document ? document.value()+"#"+text(at,"pointer") : "";
    }
    static String document(JsonValue origin) {
        JsonValue serving=get(get(origin,"document_base"),"source");
        if(get(serving,"document") instanceof JsonString value)return value.value();
        JsonValue at=get(get(get(origin,"source"),"terminal"),"source");
        if (at==JsonNull.INSTANCE) at=get(get(origin,"default_from"),"source");
        return at==JsonNull.INSTANCE?"":text(at,"document");
    }
}
