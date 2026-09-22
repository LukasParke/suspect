import example.sdk.*;
import static example.sdk.JsonRuntime.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

/** Independent JVM reflection check of the canonical Java compatibility record. */
public final class NativeMetadata {
    private NativeMetadata() {}
    private static int checks;
    private static void check(boolean value, String message) {
        if (!value) throw new AssertionError(message);
        checks++;
    }
    private static JsonObject object(JsonValue value) { return (JsonObject) value; }
    private static JsonValue get(JsonValue object, String key) { return object(object).values().getOrDefault(key, JsonNull.INSTANCE); }
    private static String text(JsonValue value) { return ((JsonString) value).value(); }
    private static String str(JsonValue object, String key) { return text(get(object, key)); }
    private static boolean flag(JsonValue object, String key) { return get(object, key) instanceof JsonBoolean value && value.value(); }
    private static List<JsonValue> array(JsonValue value) { return value instanceof JsonArray values ? values.values() : List.of(); }
    private static String canonical(Type type, Map<TypeVariable<?>, Type> bindings) {
        if (type instanceof TypeVariable<?> variable) {
            Type bound = bindings.get(variable);
            return bound == null ? variable.getName() : canonical(bound, bindings);
        }
        if (type instanceof ParameterizedType parameterized) {
            String arguments = String.join(", ", Arrays.stream(parameterized.getActualTypeArguments()).map(arg -> canonical(arg, bindings)).toList());
            return canonical(parameterized.getRawType(), bindings) + "<" + arguments + ">";
        }
        if (type instanceof GenericArrayType generic) return canonical(generic.getGenericComponentType(), bindings) + "[]";
        if (type instanceof Class<?> value) return value.isArray() ? canonical(value.getComponentType(), bindings) + "[]" : value.getName().replace('$', '.');
        return type.getTypeName().replace('$', '.');
    }
    private static String expected(JsonValue value) {
        String kind = str(value, "kind");
        if (kind.equals("nullable")) return expected(get(value, "type"));
        if (kind.equals("generic")) return str(value, "name") + "<" + String.join(", ", array(get(value, "arguments")).stream().map(NativeMetadata::expected).toList()) + ">";
        return str(value, "name");
    }
    private static Class<?> type(String name) throws ClassNotFoundException {
        String candidate = name;
        for (;;) {
            try { return Class.forName(candidate); }
            catch (ClassNotFoundException missing) {
                int dot = candidate.lastIndexOf('.');
                if (dot < 0) throw missing;
                candidate = candidate.substring(0, dot) + "$" + candidate.substring(dot + 1);
            }
        }
    }
    private static void method(Class<?> owner, JsonValue descriptor, Map<TypeVariable<?>, Type> bindings) {
        if (descriptor == JsonNull.INSTANCE) return;
        String name = str(descriptor, "name");
        List<String> parameters = array(get(descriptor, "parameters")).stream().map(p -> expected(get(p, "type"))).toList();
        List<Method> matching = Arrays.stream(owner.getMethods()).filter(m -> !m.isBridge() && !m.isSynthetic() && m.getName().equals(name))
                .filter(m -> Arrays.stream(m.getGenericParameterTypes()).map(t -> canonical(t, bindings)).toList().equals(parameters)).toList();
        check(matching.size() == 1, owner.getName() + "." + name + parameters + " missing or ambiguous");
        Method actual = matching.getFirst();
        check(Modifier.isPublic(actual.getModifiers()), "non-public method " + actual);
        check(Modifier.isStatic(actual.getModifiers()) == flag(descriptor, "static"), "static/instance mismatch " + actual);
        check(canonical(actual.getGenericReturnType(), bindings).equals(expected(get(descriptor, "returns"))), "return type mismatch " + actual + " expected " + expected(get(descriptor, "returns")));
    }
    private static void constructor(Class<?> owner, JsonValue descriptor) {
        List<String> parameters = array(get(descriptor, "parameters")).stream().map(p -> expected(get(p, "type"))).toList();
        check(Arrays.stream(owner.getConstructors()).anyMatch(c -> Arrays.stream(c.getGenericParameterTypes()).map(t -> canonical(t, Map.of())).toList().equals(parameters)), "public constructor missing for " + owner + parameters);
    }
    private static void factory(JsonValue descriptor) throws Exception {
        if (descriptor == JsonNull.INSTANCE) return;
        JsonObject normalized = object(descriptor);
        var fields = new LinkedHashMap<>(normalized.values()); fields.put("name", get(descriptor, "member"));
        method(type(str(descriptor, "owner")), new JsonObject(fields), Map.of());
    }
    private static void properties(Class<?> owner, JsonValue descriptor) throws Exception {
        for (JsonValue property : array(get(descriptor, "fields"))) {
            method(owner, get(property, "getter"), Map.of());
            String member = get(property, "name") instanceof JsonString name ? name.value() : str(property, "member");
            if (get(property, "storage") != JsonNull.INSTANCE) {
                Field field = owner.getDeclaredField(member);
                check(Modifier.isPrivate(field.getModifiers()) && Modifier.isFinal(field.getModifiers()), "model/input storage must be private final: " + field);
            }
        }
    }
    private static void builder(JsonValue descriptor) throws Exception {
        if (descriptor == JsonNull.INSTANCE) return;
        Class<?> owner = type(str(descriptor, "name"));
        check(Modifier.isPublic(owner.getModifiers()) && Modifier.isFinal(owner.getModifiers()) && Modifier.isStatic(owner.getModifiers()), "builder modifiers " + owner);
        check(owner.getConstructors().length == 0, "builder has public constructors " + owner);
        for (JsonValue setter : array(get(descriptor, "setters"))) method(owner, setter, Map.of());
        for (JsonValue omitter : array(get(descriptor, "omitters"))) method(owner, omitter, Map.of());
        method(owner, get(descriptor, "build"), Map.of());
    }
    private static void codec(JsonValue nativeModel) throws Exception {
        JsonValue descriptor = get(nativeModel, "descriptor");
        Class<?> owner = type(str(descriptor, "owner"));
        Field field = owner.getField(str(descriptor, "member"));
        check(Modifier.isPublic(field.getModifiers()) && Modifier.isStatic(field.getModifiers()) && Modifier.isFinal(field.getModifiers()), "CODEC must be public static final");
        check(canonical(field.getGenericType(), Map.of()).equals(expected(get(descriptor, "type"))), "CODEC generic type mismatch: " + field);
        ParameterizedType generic = (ParameterizedType) field.getGenericType();
        Class<?> runtime = (Class<?>) generic.getRawType();
        Map<TypeVariable<?>, Type> bindings = Map.of(runtime.getTypeParameters()[0], generic.getActualTypeArguments()[0]);
        for (JsonValue method : array(get(descriptor, "methods"))) method(runtime, method, bindings);
        JsonValue source = get(nativeModel, "source");
        String expected = str(source, "document") + "#" + str(source, "pointer");
        Object actual = runtime.getMethod("source").invoke(field.get(null));
        check(expected.equals(actual), "codec bound to the wrong source: " + field);
    }
    private static void model(JsonValue nativeModel) throws Exception {
        JsonValue descriptor = get(nativeModel, "descriptor");
        Class<?> owner = type(str(nativeModel, "name"));
        String kind = str(descriptor, "kind");
        check(Modifier.isPublic(owner.getModifiers()), "model not public " + owner);
        if (kind.equals("sealed-interface")) check(owner.isInterface() && owner.isSealed(), "union is not sealed " + owner);
        else if (kind.equals("sealed-media-class")) check(owner.isSealed() && Modifier.isAbstract(owner.getModifiers()), "media alternatives are not sealed " + owner);
        else check(Modifier.isFinal(owner.getModifiers()), "value class not final " + owner);
        for (JsonValue contract : array(get(descriptor, "implements"))) check(Arrays.stream(owner.getInterfaces()).anyMatch(i -> canonical(i, Map.of()).equals(text(contract))), "union arm interface mismatch " + owner);
        if (kind.equals("sealed-interface") || kind.equals("sealed-media-class")) {
            List<String> variants = array(get(descriptor, "variants")).stream().map(v -> str(v, "name")).sorted().toList();
            check(Arrays.stream(owner.getPermittedSubclasses()).map(c -> canonical(c, Map.of())).sorted().toList().equals(variants), "sealed union members mismatch " + owner);
        }
        if (kind.equals("sealed-media-class")) {
            for (JsonValue variant : array(get(descriptor, "variants"))) {
                Class<?> subtype = type(str(variant, "name"));
                check(Modifier.isPublic(subtype.getModifiers()) && Modifier.isStatic(subtype.getModifiers()) && Modifier.isFinal(subtype.getModifiers()), "media member modifiers " + subtype);
                check(subtype.getSuperclass().equals(owner), "media member base " + subtype);
                for (JsonValue signature : array(get(variant, "constructors"))) constructor(subtype, signature);
                if (str(variant, "constructorAccess").equals("package")) check(subtype.getConstructors().length == 0, "response media constructor exposed " + subtype);
                properties(subtype, variant);
            }
        }
        if (get(descriptor, "constructorAccess") instanceof JsonString access && access.value().equals("private")) check(owner.getConstructors().length == 0, "private model constructor exposed " + owner);
        JsonValue constructor = get(descriptor, "constructor");
        if (constructor != JsonNull.INSTANCE) {
            if (get(constructor, "style").equals(new JsonString("builder-factory"))) factory(constructor);
            else constructor(owner, constructor);
        }
        properties(owner, descriptor); builder(get(descriptor, "builder"));
        for (JsonValue convenience : array(get(descriptor, "conveniences"))) method(owner, convenience, Map.of());
        for (JsonValue literal : array(get(descriptor, "constants"))) {
            Field field = owner.getField(str(literal, "name"));
            check(Modifier.isPublic(field.getModifiers()) && Modifier.isStatic(field.getModifiers()) && Modifier.isFinal(field.getModifiers()), "literal modifiers " + field);
            Object constant = field.get(null);
            JsonValue value = (JsonValue) owner.getMethod("wireValue").invoke(constant);
            check(value.equals(get(literal, "value")), "native literal differs from retained descriptor " + field);
        }
    }
    private static void controls(JsonValue controls) throws Exception {
        JsonValue options = get(controls, "options");
        Class<?> optionsClass = type(str(options, "name"));
        check(optionsClass.getConstructors().length == 0, "options constructor must be private");
        method(optionsClass, get(options, "factory"), Map.of());
        JsonValue builder = get(options, "builder"); Class<?> builderClass = type(str(builder, "name"));
        for (JsonValue method : array(get(builder, "methods"))) method(builderClass, method, Map.of());
        method(builderClass, get(builder, "build"), Map.of());
        for (String name : List.of("codecLimits", "jsonLimits")) {
            JsonValue limits = get(controls, name); Class<?> owner = type(str(limits, "name"));
            check(owner.isRecord(), "limits are not records " + owner); constructor(owner, get(limits, "constructor"));
            for (JsonValue method : array(get(limits, "methods"))) method(owner, method, Map.of());
        }
        JsonValue failure = get(controls, "failure"); Class<?> exception = type(str(failure, "name"));
        for (JsonValue method : array(get(failure, "methods"))) method(exception, method, Map.of());
    }
    private static void operation(JsonValue operation) throws Exception {
        JsonValue descriptor = get(operation, "descriptor"), symbols = get(operation, "symbols");
        Class<?> client = type(str(symbols, "client"));
        JsonValue construction = get(descriptor, "constructor"); factory(get(construction, "input"));
        JsonValue input = get(construction, "inputDeclaration"); Class<?> inputClass = type(str(input, "name"));
        check(Modifier.isStatic(inputClass.getModifiers()) && Modifier.isFinal(inputClass.getModifiers()), "input must be nested static final");
        check(inputClass.getConstructors().length == 0, "input exposed a constructor");
        properties(inputClass, input); builder(get(input, "builder"));
        Class<?> inputBuilder = type(str(get(input, "builder"), "name"));
        for (JsonValue property : array(get(input, "fields"))) {
            method(inputBuilder, get(property, "builderSetter"), Map.of()); method(inputBuilder, get(property, "omitter"), Map.of());
        }
        JsonValue clientDescriptor = get(construction, "client"); constructor(client, get(clientDescriptor, "constructor"));
        method(client, get(clientDescriptor, "close"), Map.of()); controls(get(clientDescriptor, "controls"));
        JsonValue methods = get(descriptor, "parameters");
        for (String name : List.of("method", "asyncMethod", "noInputMethod", "noInputAsyncMethod")) method(client, get(methods, name), Map.of());
        JsonValue credential = get(descriptor, "credential"); method(type(str(credential, "owner")), get(credential, "method"), Map.of());
        JsonValue success = get(get(descriptor, "responseUnions"), "success");
        if (str(success, "kind").equals("sealed-interface")) {
            Class<?> result = type(str(success, "name"));
            check(result.isInterface() && result.isSealed(), "success alternatives are not sealed");
            check(Arrays.stream(result.getPermittedSubclasses()).map(c -> canonical(c, Map.of())).sorted().toList()
                    .equals(array(get(success, "members")).stream().map(NativeMetadata::text).sorted().toList()), "success member identity mismatch");
        }
        for (JsonValue response : array(get(descriptor, "responses"))) {
            Class<?> owner = type(str(response, "type"));
            check(owner.getConstructors().length == 0, "status constructor exposed " + owner);
            check(Modifier.isPublic(owner.getModifiers()) && Modifier.isFinal(owner.getModifiers()) && Modifier.isStatic(owner.getModifiers()), "status class modifiers " + owner);
            check(canonical(owner.getSuperclass(), Map.of()).equals(str(response, "base")), "status base " + owner);
            for (JsonValue contract : array(get(response, "implements"))) check(Arrays.stream(owner.getInterfaces()).anyMatch(i -> canonical(i, Map.of()).equals(text(contract))), "status interface " + owner);
            properties(owner, response); method(owner, get(response, "statusGetter"), Map.of());
        }
    }
    public static void main(String[] args) throws Exception {
        JsonObject snapshot = object(JsonRuntime.parse(Files.readAllBytes(Path.of(args[0]))));
        for (JsonValue value : array(get(snapshot, "models"))) {
            if (str(value, "role").equals("codec")) codec(value); else model(value);
        }
        for (JsonValue operation : array(get(snapshot, "operations"))) operation(operation);
        System.out.println("JAVA_NATIVE_COMPATIBILITY_METADATA_OK: " + checks + " reflection/source-codec/literal assertions");
    }
}
