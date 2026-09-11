package {package};

import java.util.Arrays;
import java.util.Objects;

/** Immutable native octets. Bytes are never represented as JSON null or a filename. */
public final class Bytes {
    private final byte[] value;
    private Bytes(byte[] value) { this.value = value; }
    /** Snapshot caller-owned bytes. @param value octets @return immutable bytes */
    public static Bytes of(byte[] value) {
        Objects.requireNonNull(value);
        if (value.length > JsonRuntime.MAX_BYTES) throw new IllegalArgumentException("byte allocation ceiling");
        return new Bytes(value.clone());
    }
    /** Empty octets. @return immutable empty value */
    public static Bytes empty() { return new Bytes(new byte[0]); }
    /** Number of octets. @return size */
    public int size() { return value.length; }
    /** A defensive copy. @return octets */
    public byte[] toByteArray() { return value.clone(); }
    static Bytes owned(byte[] value) { return new Bytes(value); }
    byte[] internal() { return value; }
    @Override public boolean equals(Object other) { return other instanceof Bytes bytes && Arrays.equals(value, bytes.value); }
    @Override public int hashCode() { return Arrays.hashCode(value); }
    @Override public String toString() { return "Bytes(" + value.length + ")"; }
}
