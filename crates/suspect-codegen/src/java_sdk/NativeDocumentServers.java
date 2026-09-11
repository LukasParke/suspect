import example.documentservers.*;
import java.net.URI;

/** Retrieval-document server resolution, independent of $self/schema identity. */
public final class NativeDocumentServers {
    private NativeDocumentServers() {}
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    public static void main(String[] args) {
        var transport = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", "true"));
        try (var client = new Client(HttpRuntime.Options.builder().httpClient(transport).build())) {
            check(client.inherited().data(), "inherited body");
            check(transport.requests.getLast().uri().equals("https://entry.example.test/inherited"), "absent root servers must use entry retrieval document");
            client.explicitDefault();
            check(transport.requests.getLast().uri().equals("https://parts.example.test/explicit"), "explicit empty servers belong to their serving document");
            client.relative();
            check(transport.requests.getLast().uri().equals("https://parts.example.test/wire/relative"), "external server relative to physical serving URL");
            client.relative(RequestOptions.builder().documentUrl(URI.create("https://override.example.test/specs/api.json")).build());
            check(transport.requests.getLast().uri().equals("https://override.example.test/wire/relative"), "explicit retrieval override");
        }
        System.out.println("JAVA_DOCUMENT_SERVERS_OK: entry defaults, external empty/relative servers, $self and caller override");
    }
}
