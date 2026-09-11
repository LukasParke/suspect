package {package};

/** Optional presence independent of nullability. A present null differs from absence. */
public final class Presence<T> {
    private final boolean present;
    private final T value;
    private Presence(boolean present, T value) { this.present = present; this.value = value; }
    public static <T> Presence<T> absent() { return new Presence<>(false, null); }
    public static <T> Presence<T> of(T value) { return new Presence<>(true, value); }
    public boolean isPresent() { return present; }
    public T value() { if (!present) throw new IllegalStateException("value is absent"); return value; }
    @Override public boolean equals(Object other) { return other instanceof Presence<?> p && present == p.present && java.util.Objects.equals(value, p.value); }
    @Override public int hashCode() { return java.util.Objects.hash(present, value); }
    @Override public String toString() { return present ? "Present" : "Absent"; }
}
