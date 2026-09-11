package {package};

/** A source-bound schema mismatch or explicit resource/evaluation failure. */
public final class CodecException extends IllegalArgumentException {
    private static final long serialVersionUID = 1L;
    private final String kind;
    private final String source;
    private final String instancePath;
    public CodecException(String kind, String source, String instancePath, String message) {
        super(kind + " at " + safe(source) + " " + safe(instancePath) + ": " + safe(message));
        this.kind = kind; this.source = source; this.instancePath = instancePath;
    }
    public String kind() { return kind; }
    public String source() { return source; }
    public String instancePath() { return instancePath; }
    private static String safe(String value) {
        if (value == null) return "";
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < value.length() && out.length() < 240; i++) {
            char c = value.charAt(i);
            if (c < 32 || c == 127 || c == '\\' || c == '"') {
                out.append('\\').append('u');
                for (int shift = 12; shift >= 0; shift -= 4) out.append("0123456789abcdef".charAt((c >> shift) & 15));
            } else out.append(c);
        }
        if (value.length() > 240) out.append("...");
        return out.toString();
    }
}
