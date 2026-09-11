package {package};

import java.util.*;
import static {package}.JsonRuntime.*;

/** Private transport values. Opaque bytes are a separate domain from JSON. */
sealed interface WireValue {
    record Scalar(JsonValue value) implements WireValue { public Scalar { Objects.requireNonNull(value); } }
    record Binary(Bytes value) implements WireValue { public Binary { Objects.requireNonNull(value); } }
    record Items(List<JsonValue> values) implements WireValue { public Items { values=List.copyOf(values); } }
    record Part(WireValue value,String contentType,String filename,Map<String,JsonValue> headers) implements WireValue {
        public Part { Objects.requireNonNull(value); headers=Map.copyOf(headers); }
    }
    record Parts(Map<String,List<Part>> named,List<Part> positional) implements WireValue {
        public Parts { var copy=new LinkedHashMap<String,List<Part>>(); named.forEach((k,v)->copy.put(k,List.copyOf(v))); named=Collections.unmodifiableMap(copy); positional=List.copyOf(positional); }
    }
    record Selected(String declaration,String contentType,WireValue value) implements WireValue { public Selected { Objects.requireNonNull(declaration); Objects.requireNonNull(contentType); Objects.requireNonNull(value); } }
}
