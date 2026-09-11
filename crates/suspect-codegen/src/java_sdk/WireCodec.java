package {package};

import java.util.*;
import java.util.function.*;
import static {package}.JsonRuntime.*;

/** Generated typed wire binding. Aggregate/byte schemas never become JSON roots. */
final class WireCodec<T> {
    private final String source;
    private final BiFunction<WireValue,ModelCodec.Context,T> reader;
    private final BiFunction<T,ModelCodec.Context,WireValue> writer;
    WireCodec(String source,BiFunction<WireValue,ModelCodec.Context,T> reader,BiFunction<T,ModelCodec.Context,WireValue> writer) { this.source=source; this.reader=reader; this.writer=writer; }
    T read(WireValue value,ModelCodec.Context c) { return guard(c,()->reader.apply(value,c)); }
    WireValue write(T value,ModelCodec.Context c) { return guard(c,()->writer.apply(value,c)); }
    T snapshot(T value) { return snapshot(value,new ModelCodec.Context()); }
    T snapshot(T value,ModelCodec.Context c) { return guard(c,()->reader.apply(writer.apply(value,c),c)); }
    private <V> V guard(ModelCodec.Context c,Supplier<V> action) {
        c.spend(1);
        try { return action.get(); }
        catch (ClassCastException|NullPointerException error) { throw new CodecException("invalid",source,c.path(),"native wire representation mismatch"); }
    }
    static <T> WireCodec<T> model(ModelCodec<T> codec) { return new WireCodec<>(codec.source(),(v,c)->codec.decodeValue(((WireValue.Scalar)v).value(),c),(v,c)->new WireValue.Scalar(codec.encodeValue(v,c))); }
    static WireCodec<JsonValue> json() { return new WireCodec<>("",(v,c)->c.json(((WireValue.Scalar)v).value()),(v,c)->new WireValue.Scalar(c.json(v))); }
    static WireCodec<String> text() { return new WireCodec<>("",(v,c)->c.string(((JsonString)((WireValue.Scalar)v).value()).value()),(v,c)->new WireValue.Scalar(new JsonString(c.string(v)))); }
    static WireCodec<Bytes> bytes() { return new WireCodec<>("",(v,c)->bytes(((WireValue.Binary)v).value(),c),(v,c)->new WireValue.Binary(bytes(v,c))); }
    static Bytes bytes(Bytes value,ModelCodec.Context c) { Objects.requireNonNull(value); c.spend(value.size()); return value; }
    static <T> WireCodec<List<T>> items(ModelCodec<T> codec) { return new WireCodec<>(codec.source(),(v,c)->{
        var values=new ArrayList<T>(); for(JsonValue item:((WireValue.Items)v).values()) { c.spend(1); values.add(codec.decodeValue(item,c)); } return Collections.unmodifiableList(values);
    },(v,c)->{var values=new ArrayList<JsonValue>();for(T item:v){c.spend(1);values.add(codec.encodeValue(item,c));}return new WireValue.Items(values);}); }
}
