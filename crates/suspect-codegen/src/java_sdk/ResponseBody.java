package {package};

/** A status range/default can match both content-bearing and body-forbidden statuses.
 * @param <T> the declared content representation
 */
public final class ResponseBody<T> {
    private final boolean present;
    private final T value;
    private ResponseBody(boolean present, T value) { this.present = present; this.value = value; }
    /** Whether HTTP permits this actual response body. @return body presence */
    public boolean isPresent() { return present; }
    /** Declared content, which can itself represent JSON null. @return content */
    public T value() { if (!present) throw new IllegalStateException("HTTP forbids a body for this response"); return value; }
    static <T> ResponseBody<T> content(T value) { return new ResponseBody<>(true, value); }
    static <T> ResponseBody<T> empty() { return new ResponseBody<>(false, null); }
    @Override public String toString() { return present ? "Content" : "NoContent"; }
}
