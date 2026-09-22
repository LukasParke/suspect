package {package};

/** Typed SDK failure. Body bytes are explicit bounded captures, never part of the message. */
public class SdkException extends RuntimeException {
    private static final long serialVersionUID = 1L;
    private final String kind, source, schemaSource, instancePath;
    private final int status;
    private final byte[] capture;
    private final boolean truncated;
    public SdkException(String kind, String source, int status, byte[] capture, boolean truncated) {
        this(kind, source, status, capture, truncated, "", "");
    }
    public SdkException(String kind, String source, int status, byte[] capture, boolean truncated, String schemaSource, String instancePath) {
        super("SDK " + kind + (status == 0 ? "" : " (HTTP " + status + ")"));
        this.kind = kind; this.source = source; this.status = status; this.capture = capture.clone(); this.truncated = truncated;
        this.schemaSource = schemaSource; this.instancePath = instancePath;
    }
    protected SdkException(SdkException value) { this(value.kind, value.source, value.status, value.capture, value.truncated, value.schemaSource, value.instancePath); }
    public String kind() { return kind; }
    public String source() { return source; }
    public int status() { return status; }
    public byte[] capture() { return capture.clone(); }
    public boolean truncated() { return truncated; }
    public String schemaSource() { return schemaSource; }
    public String instancePath() { return instancePath; }
}
